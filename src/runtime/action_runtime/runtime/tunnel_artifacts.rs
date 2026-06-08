use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_tunnel_control(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status")
            .to_ascii_lowercase();
        let provider = arguments
            .get("provider")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let base_url = crate::core::runtime::net::internal_api_base_url();
        let client = crate::core::runtime::net::build_internal_control_client(5)
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let endpoint = match action.as_str() {
            "start" => "/tunnel/start",
            "stop" => "/tunnel/stop",
            "status" => "/tunnel/status",
            other => {
                return Err(anyhow::anyhow!(
                    "Invalid action '{}'. Use start, stop, or status.",
                    other
                ));
            }
        };

        let mut req = match action.as_str() {
            "status" => client.get(format!("{}{}", base_url, endpoint)),
            "start" => {
                let body = match provider.as_deref() {
                    Some(value) => serde_json::json!({ "provider": value }),
                    None => serde_json::json!({}),
                };
                client.post(format!("{}{}", base_url, endpoint)).json(&body)
            }
            _ => client
                .post(format!("{}{}", base_url, endpoint))
                .json(&serde_json::json!({})),
        };

        if let Ok(mgr) =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))
        {
            if let Ok(Some(key)) = mgr.get_api_key() {
                if !key.trim().is_empty() {
                    req = req.bearer_auth(key);
                }
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reach tunnel controller: {}", e))?;
        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();
        let payload: serde_json::Value =
            serde_json::from_str(&raw_body).unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            let err = payload
                .get("error")
                .and_then(|v| v.as_str())
                .or_else(|| payload.get("message").and_then(|v| v.as_str()))
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
                .or_else(|| {
                    let trimmed = raw_body.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.chars().take(400).collect::<String>())
                    }
                })
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Err(anyhow::anyhow!("Tunnel command failed: {}", err));
        }

        match action.as_str() {
            "start" => {
                let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if !url.is_empty() {
                    Ok(format!("Tunnel started.\nExternal URL: {}", url))
                } else {
                    Ok(payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Tunnel is starting; URL pending.")
                        .to_string())
                }
            }
            "stop" => Ok(payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Tunnel stopped.")
                .to_string()),
            _ => {
                let active = payload
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut out = format!(
                    "Tunnel status: {}",
                    if active { "active" } else { "inactive" }
                );
                if let Some(url) = payload.get("url").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        out.push_str(&format!("\nExternal URL: {}", url));
                    }
                }
                if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
                    if !err.is_empty() {
                        out.push_str(&format!("\nLast error: {}", err));
                    }
                }
                Ok(out)
            }
        }
    }

    pub(in crate::runtime) fn pdf_text_literal(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '(' => out.push_str("\\("),
                ')' => out.push_str("\\)"),
                '\t' => out.push(' '),
                '\u{00a0}' => out.push(' '),
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' => {
                    out.push('-')
                }
                '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => out.push('\''),
                '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => out.push('"'),
                '\u{2022}' | '\u{25e6}' => out.push('*'),
                '\u{2026}' => out.push_str("..."),
                ch if ch.is_ascii_graphic() || ch == ' ' => out.push(ch),
                _ => out.push(' '),
            }
        }
        out
    }

    pub(in crate::runtime) fn wrap_pdf_text(text: &str, max_chars: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for raw_line in text.lines() {
            let mut current = String::new();
            for word in raw_line.split_whitespace() {
                let separator = usize::from(!current.is_empty());
                if !current.is_empty() && current.len() + separator + word.len() > max_chars {
                    lines.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
            if current.is_empty() {
                lines.push(String::new());
            } else {
                lines.push(current);
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    pub(in crate::runtime) fn generate_simple_pdf_bytes(
        title: &str,
        content: &str,
        style: &str,
    ) -> Vec<u8> {
        const PAGE_WIDTH: usize = 612;
        const PAGE_HEIGHT: usize = 792;
        const LINES_PER_PAGE: usize = 42;
        let body_font = match style {
            "invoice" => 10,
            "report" | "letter" | "plain" => 11,
            _ => 11,
        };
        let title_font = match style {
            "invoice" => 20,
            "report" => 16,
            "letter" | "plain" => 14,
            _ => 14,
        };
        let mut lines = Vec::new();
        lines.push(title.trim().to_string());
        lines.push(String::new());
        lines.extend(Self::wrap_pdf_text(content, 92));
        let pages = lines.chunks(LINES_PER_PAGE).collect::<Vec<_>>();
        let page_count = pages.len().max(1);
        let catalog_id = 1usize;
        let pages_id = 2usize;
        let font_id = 3usize;
        let first_page_id = 4usize;
        let mut objects: Vec<String> = Vec::new();
        objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R >>\nendobj\n"
        ));
        let kids = (0..page_count)
            .map(|index| format!("{} 0 R", first_page_id + index * 2))
            .collect::<Vec<_>>()
            .join(" ");
        objects.push(format!(
            "{pages_id} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {page_count} >>\nendobj\n"
        ));
        objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n"
        ));
        for (index, page_lines) in pages.iter().enumerate() {
            let page_id = first_page_id + index * 2;
            let content_id = page_id + 1;
            let mut stream = String::from("BT\n72 740 Td\n");
            for (line_index, line) in page_lines.iter().enumerate() {
                if line_index == 0 && index == 0 {
                    stream.push_str(&format!("/F1 {title_font} Tf\n"));
                } else if line_index == 1 && index == 0 {
                    stream.push_str(&format!("/F1 {body_font} Tf\n"));
                }
                if line_index > 0 {
                    stream.push_str("0 -16 Td\n");
                }
                stream.push_str(&format!("({}) Tj\n", Self::pdf_text_literal(line)));
            }
            stream.push_str("ET\n");
            objects.push(format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>\nendobj\n"
            ));
            objects.push(format!(
                "{content_id} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                stream.len(),
                stream
            ));
        }

        let mut pdf = String::from("%PDF-1.4\n%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n");
        let mut offsets = vec![0usize];
        for object in &objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_offset = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()));
        for offset in offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len()
        ));
        pdf.into_bytes()
    }

    /// Convert HTML to plain text by stripping tags and decoding entities
    pub(in crate::runtime) fn html_to_text(html: &str) -> String {
        // Remove script and style blocks entirely
        let script_re =
            regex::Regex::new(r"(?is)<(script|style|noscript)[^>]*>.*?</(script|style|noscript)>")
                .unwrap();
        let cleaned = script_re.replace_all(html, "");

        // Remove HTML comments
        let comment_re = regex::Regex::new(r"(?s)<!--.*?-->").unwrap();
        let cleaned = comment_re.replace_all(&cleaned, "");

        // Replace block-level elements with newlines
        let block_re = regex::Regex::new(r"(?i)</(p|div|h[1-6]|li|tr|br|hr)[^>]*>").unwrap();
        let cleaned = block_re.replace_all(&cleaned, "\n");
        let br_re = regex::Regex::new(r"(?i)<br[^>]*/?>").unwrap();
        let cleaned = br_re.replace_all(&cleaned, "\n");

        // Strip all remaining HTML tags
        let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
        let cleaned = tag_re.replace_all(&cleaned, "");

        // Decode common HTML entities
        let text = cleaned
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
            .replace("&#x27;", "'")
            .replace("&#x2F;", "/");

        // Collapse multiple whitespace/newlines
        let ws_re = regex::Regex::new(r"[ \t]+").unwrap();
        let text = ws_re.replace_all(&text, " ");
        let nl_re = regex::Regex::new(r"\n{3,}").unwrap();
        let text = nl_re.replace_all(&text, "\n\n");

        // Trim lines and overall result
        let text: String = text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate to reasonable size (10000 chars)
        if text.len() > 10000 {
            format!(
                "{}...\n\n(content truncated at 10000 characters)",
                &text[..10000]
            )
        } else {
            text.trim().to_string()
        }
    }
}
