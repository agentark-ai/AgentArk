use super::super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfChartKind {
    Bar,
    Line,
    Area,
    Scatter,
    Pie,
    Doughnut,
}

#[derive(Debug, Clone)]
struct PdfChartRow {
    label: String,
    values: Vec<f64>,
}

#[derive(Debug, Clone)]
struct PdfChart {
    title: String,
    subtitle: String,
    kind: PdfChartKind,
    series_names: Vec<String>,
    rows: Vec<PdfChartRow>,
}

#[derive(Debug, Clone)]
enum PdfRenderElement {
    Text(String),
    Chart(PdfChart),
}

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

    fn pdf_markdown_link_display(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(open_label) = rest.find('[') {
            let before = &rest[..open_label];
            let after_open = &rest[open_label + 1..];
            let Some(close_label) = after_open.find(']') else {
                break;
            };
            let after_label = &after_open[close_label + 1..];
            if !after_label.starts_with('(') {
                out.push_str(before);
                out.push('[');
                rest = after_open;
                continue;
            }
            let after_open_url = &after_label[1..];
            let Some(close_url) = after_open_url.find(')') else {
                break;
            };
            let label = &after_open[..close_label];
            let url = &after_open_url[..close_url];
            out.push_str(before);
            out.push_str(label.trim());
            if !url.trim().is_empty() {
                out.push_str(" (");
                out.push_str(url.trim());
                out.push(')');
            }
            rest = &after_open_url[close_url + 1..];
        }
        out.push_str(rest);
        out
    }

    pub(in crate::runtime) fn pdf_display_text_line(raw_line: &str) -> String {
        let trimmed_end = raw_line.trim_end();
        let trimmed = trimmed_end.trim_start();
        if trimmed.len() >= 3
            && trimmed
                .chars()
                .all(|ch| matches!(ch, '-' | '*' | '_' | ' '))
            && trimmed.chars().any(|ch| matches!(ch, '-' | '*' | '_'))
        {
            return String::new();
        }

        let mut line = trimmed_end.to_string();
        let trimmed = line.trim_start();
        let heading_marks = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&heading_marks) {
            let after_marks = &trimmed[heading_marks..];
            if after_marks.chars().next().is_some_and(char::is_whitespace) {
                line = after_marks.trim_start().to_string();
            }
        }

        Self::pdf_markdown_link_display(&line)
            .replace("***", "")
            .replace("**", "")
            .replace("__", "")
            .replace('`', "")
    }

    fn pdf_wrap_display_line(raw_line: &str, max_chars: usize) -> Vec<String> {
        let raw_line = Self::pdf_display_text_line(raw_line);
        let mut lines = Vec::new();
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
        lines
    }

    fn pdf_chart_text(value: &serde_json::Value, fallback: &str) -> String {
        match value {
            serde_json::Value::String(text) if !text.trim().is_empty() => text.trim().to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Bool(value) => value.to_string(),
            _ => fallback.to_string(),
        }
    }

    fn pdf_chart_number(value: &serde_json::Value) -> Option<f64> {
        match value {
            serde_json::Value::Number(number) => number.as_f64(),
            serde_json::Value::String(text) => Self::pdf_number_from_text(text),
            _ => None,
        }
    }

    fn pdf_numbers_in_text(text: &str) -> Vec<f64> {
        let mut values = Vec::new();
        let mut token = String::new();
        let mut saw_digit = false;
        let flush = |token: &mut String, saw_digit: &mut bool, values: &mut Vec<f64>| {
            if *saw_digit {
                let cleaned = token.replace(',', "");
                if let Ok(value) = cleaned.parse::<f64>() {
                    if value.is_finite() {
                        values.push(value);
                    }
                }
            }
            token.clear();
            *saw_digit = false;
        };

        for ch in text.chars() {
            if ch.is_ascii_digit() {
                token.push(ch);
                saw_digit = true;
            } else if (ch == '.' || ch == ',') && saw_digit {
                token.push(ch);
            } else {
                flush(&mut token, &mut saw_digit, &mut values);
            }
        }
        flush(&mut token, &mut saw_digit, &mut values);
        values
    }

    fn pdf_number_from_text(text: &str) -> Option<f64> {
        let values = Self::pdf_numbers_in_text(text);
        if values.is_empty() {
            return None;
        }
        let lower = text.to_ascii_lowercase();
        let range_like =
            (text.contains('-') || text.contains('\u{2013}') || lower.contains(" to "))
                && values.len() >= 2;
        if range_like {
            Some((values[0] + values[1]) / 2.0)
        } else {
            values.first().copied()
        }
    }

    fn pdf_chart_kind(value: &serde_json::Value) -> PdfChartKind {
        match Self::pdf_chart_text(value, "bar")
            .to_ascii_lowercase()
            .as_str()
        {
            "line" => PdfChartKind::Line,
            "area" => PdfChartKind::Area,
            "scatter" => PdfChartKind::Scatter,
            "pie" => PdfChartKind::Pie,
            "doughnut" | "donut" => PdfChartKind::Doughnut,
            _ => PdfChartKind::Bar,
        }
    }

    fn pdf_parse_chart_block(raw_json: &str) -> Option<PdfChart> {
        let spec = serde_json::from_str::<serde_json::Value>(raw_json).ok()?;
        let object = spec.as_object()?;
        let rows = object.get("data")?.as_array()?;
        let first_row = rows.iter().find_map(|row| row.as_object())?;
        let category_key = object
            .get("x")
            .map(|value| Self::pdf_chart_text(value, ""))
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                first_row
                    .iter()
                    .find(|(_, value)| Self::pdf_chart_number(value).is_none())
                    .map(|(key, _)| key.clone())
            })
            .or_else(|| first_row.keys().next().cloned())?;

        let mut series_keys = Vec::new();
        let mut series_names = Vec::new();
        if let Some(series) = object.get("series").and_then(|value| value.as_array()) {
            for item in series {
                if let Some(key) = item.as_str().map(str::trim).filter(|key| !key.is_empty()) {
                    series_keys.push(key.to_string());
                    series_names.push(key.to_string());
                    continue;
                }
                if let Some(series_object) = item.as_object() {
                    let key = series_object
                        .get("key")
                        .map(|value| Self::pdf_chart_text(value, ""))
                        .filter(|value| !value.trim().is_empty());
                    if let Some(key) = key {
                        let name = series_object
                            .get("name")
                            .map(|value| Self::pdf_chart_text(value, &key))
                            .unwrap_or_else(|| key.clone());
                        series_keys.push(key);
                        series_names.push(name);
                    }
                }
            }
        }
        if series_keys.is_empty() {
            for key in first_row.keys() {
                if key != &category_key
                    && rows.iter().any(|row| {
                        row.as_object()
                            .and_then(|object| object.get(key))
                            .and_then(Self::pdf_chart_number)
                            .is_some()
                    })
                {
                    series_keys.push(key.clone());
                    series_names.push(key.clone());
                }
                if series_keys.len() >= 4 {
                    break;
                }
            }
        }
        if series_keys.is_empty() {
            return None;
        }

        let parsed_rows = rows
            .iter()
            .take(28)
            .filter_map(|row| {
                let row = row.as_object()?;
                let label = row
                    .get(&category_key)
                    .map(|value| Self::pdf_chart_text(value, ""))
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("Row {}", row.len()));
                let values = series_keys
                    .iter()
                    .map(|key| row.get(key).and_then(Self::pdf_chart_number).unwrap_or(0.0))
                    .collect::<Vec<_>>();
                Some(PdfChartRow { label, values })
            })
            .collect::<Vec<_>>();
        if parsed_rows.is_empty() {
            return None;
        }

        Some(PdfChart {
            title: object
                .get("title")
                .map(|value| Self::pdf_chart_text(value, "Chart"))
                .unwrap_or_else(|| "Chart".to_string()),
            subtitle: object
                .get("subtitle")
                .map(|value| Self::pdf_chart_text(value, ""))
                .unwrap_or_default(),
            kind: object
                .get("type")
                .map(Self::pdf_chart_kind)
                .unwrap_or(PdfChartKind::Bar),
            series_names,
            rows: parsed_rows,
        })
    }

    fn pdf_has_chart_fence(content: &str) -> bool {
        content.lines().any(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("```agentark-chart")
        })
    }

    fn pdf_markdown_table_cells(line: &str) -> Vec<String> {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect()
    }

    fn pdf_is_markdown_table_separator(cells: &[String]) -> bool {
        !cells.is_empty()
            && cells.iter().all(|cell| {
                let trimmed = cell.trim();
                !trimmed.is_empty()
                    && trimmed.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
                    && trimmed.chars().any(|ch| ch == '-')
            })
    }

    fn pdf_heading_from_line(line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let display = Self::pdf_display_text_line(trimmed);
            return (!display.trim().is_empty()).then_some(display);
        }
        if trimmed.len() <= 120
            && trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            && trimmed.contains(". ")
        {
            return Some(Self::pdf_display_text_line(trimmed));
        }
        None
    }

    fn pdf_chart_from_markdown_table(
        heading: Option<&str>,
        table_lines: &[String],
    ) -> Option<PdfChart> {
        let mut table_rows = table_lines
            .iter()
            .map(|line| Self::pdf_markdown_table_cells(line))
            .filter(|cells| !Self::pdf_is_markdown_table_separator(cells))
            .collect::<Vec<_>>();
        if table_rows.len() < 3 {
            return None;
        }
        let headers = table_rows.remove(0);
        if headers.len() < 2 {
            return None;
        }
        let label_col = 0usize;
        let mut best_numeric_col = None;
        let mut best_count = 0usize;
        for col in 1..headers.len() {
            let count = table_rows
                .iter()
                .filter(|row| {
                    row.get(col)
                        .and_then(|value| Self::pdf_number_from_text(value))
                        .is_some()
                })
                .count();
            if count > best_count {
                best_count = count;
                best_numeric_col = Some(col);
            }
        }
        let numeric_col = best_numeric_col?;
        if best_count < 2 {
            return None;
        }

        let rows = table_rows
            .iter()
            .filter_map(|row| {
                let label = row
                    .get(label_col)
                    .map(|value| Self::pdf_display_text_line(value))
                    .filter(|value| !value.trim().is_empty())?;
                let value = row
                    .get(numeric_col)
                    .and_then(|value| Self::pdf_number_from_text(value))?;
                Some(PdfChartRow {
                    label,
                    values: vec![value],
                })
            })
            .take(10)
            .collect::<Vec<_>>();
        if rows.len() < 2 {
            return None;
        }

        let title = heading
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{} by {}",
                    headers
                        .get(label_col)
                        .map(String::as_str)
                        .unwrap_or("Category"),
                    headers
                        .get(numeric_col)
                        .map(String::as_str)
                        .unwrap_or("Value")
                )
            });
        let series_name = headers
            .get(numeric_col)
            .map(|value| Self::pdf_display_text_line(value))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Value".to_string());
        let positive_total = rows
            .iter()
            .filter_map(|row| row.values.first().copied())
            .filter(|value| *value > 0.0)
            .sum::<f64>();
        let kind = if rows.len() <= 8
            && positive_total > 0.0
            && (80.0..=120.0).contains(&positive_total)
            && (title.to_ascii_lowercase().contains("share")
                || title.to_ascii_lowercase().contains("scope")
                || series_name.to_ascii_lowercase().contains("share")
                || series_name.contains('%'))
        {
            PdfChartKind::Doughnut
        } else {
            PdfChartKind::Bar
        };

        Some(PdfChart {
            title,
            subtitle: format!("Auto-generated from report table: {}", series_name),
            kind,
            series_names: vec![series_name],
            rows,
        })
    }

    fn pdf_auto_chart_elements(content: &str) -> Vec<PdfRenderElement> {
        if Self::pdf_has_chart_fence(content) {
            return Vec::new();
        }

        let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
        let lines = normalized.lines().collect::<Vec<_>>();
        let mut heading: Option<String> = None;
        let mut charts = Vec::new();
        let mut index = 0usize;
        while index < lines.len() {
            let line = lines[index];
            if let Some(candidate) = Self::pdf_heading_from_line(line) {
                heading = Some(candidate);
            }
            if !line.trim_start().starts_with('|') {
                index += 1;
                continue;
            }

            let mut table_lines = Vec::new();
            while index < lines.len() && lines[index].trim_start().starts_with('|') {
                table_lines.push(lines[index].to_string());
                index += 1;
            }
            if let Some(chart) =
                Self::pdf_chart_from_markdown_table(heading.as_deref(), &table_lines)
            {
                charts.push(chart);
                if charts.len() >= 4 {
                    break;
                }
            }
        }

        if charts.is_empty() {
            return Vec::new();
        }

        let mut elements = vec![
            PdfRenderElement::Text("Visual Summary".to_string()),
            PdfRenderElement::Text(
                "Generated automatically from numeric tables in this report.".to_string(),
            ),
            PdfRenderElement::Text(String::new()),
        ];
        elements.extend(charts.into_iter().map(PdfRenderElement::Chart));
        elements.push(PdfRenderElement::Text(String::new()));
        elements
    }

    fn pdf_render_elements(content: &str, max_chars: usize) -> Vec<PdfRenderElement> {
        let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
        let mut elements = Vec::new();
        let mut lines = normalized.lines();
        while let Some(raw_line) = lines.next() {
            let trimmed = raw_line.trim_start();
            if let Some(language) = trimmed.strip_prefix("```") {
                let language = language.trim().to_ascii_lowercase();
                let mut code_lines = Vec::new();
                for code_line in lines.by_ref() {
                    if code_line.trim_start().starts_with("```") {
                        break;
                    }
                    code_lines.push(code_line);
                }
                if language == crate::core::platform::inline_artifacts::INLINE_CHART_FENCE_LANGUAGE
                {
                    if let Some(chart) = Self::pdf_parse_chart_block(&code_lines.join("\n")) {
                        elements.push(PdfRenderElement::Chart(chart));
                        continue;
                    }
                }
                for code_line in code_lines {
                    for wrapped in Self::pdf_wrap_display_line(code_line, max_chars) {
                        elements.push(PdfRenderElement::Text(wrapped));
                    }
                }
                continue;
            }

            for wrapped in Self::pdf_wrap_display_line(raw_line, max_chars) {
                elements.push(PdfRenderElement::Text(wrapped));
            }
        }
        if elements.is_empty() {
            elements.push(PdfRenderElement::Text(String::new()));
        }
        elements
    }

    fn pdf_add_text(stream: &mut String, x: f64, y: f64, font_size: usize, text: &str) {
        stream.push_str(&format!(
            "BT\n/F1 {font_size} Tf\n1 0 0 1 {x:.1} {y:.1} Tm\n({}) Tj\nET\n",
            Self::pdf_text_literal(text)
        ));
    }

    fn pdf_color(index: usize) -> (f64, f64, f64) {
        const COLORS: &[(f64, f64, f64)] = &[
            (0.05, 0.55, 0.34),
            (0.77, 0.47, 0.16),
            (0.33, 0.43, 0.76),
            (0.65, 0.30, 0.60),
            (0.22, 0.56, 0.62),
        ];
        COLORS[index % COLORS.len()]
    }

    fn pdf_rect(
        stream: &mut String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: (f64, f64, f64),
    ) {
        let (r, g, b) = color;
        stream.push_str(&format!(
            "{r:.3} {g:.3} {b:.3} rg\n{x:.1} {y:.1} {width:.1} {height:.1} re\nf\n"
        ));
    }

    fn pdf_line(
        stream: &mut String,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        width: f64,
        color: (f64, f64, f64),
    ) {
        let (r, g, b) = color;
        stream.push_str(&format!(
            "{r:.3} {g:.3} {b:.3} RG\n{width:.1} w\n{x1:.1} {y1:.1} m\n{x2:.1} {y2:.1} l\nS\n"
        ));
    }

    fn pdf_chart_value_label(value: f64) -> String {
        if value.abs() >= 1000.0 {
            format!("{value:.0}")
        } else if value.abs() >= 10.0 {
            format!("{value:.1}")
        } else {
            format!("{value:.2}")
        }
    }

    fn pdf_short_text(value: &str, max_chars: usize) -> String {
        let mut out = String::new();
        for ch in value.chars().take(max_chars) {
            out.push(ch);
        }
        if value.chars().count() > max_chars {
            out.push_str("...");
        }
        out
    }

    fn pdf_chart_height(chart: &PdfChart) -> f64 {
        match chart.kind {
            PdfChartKind::Bar => 76.0 + chart.rows.len().min(10) as f64 * 19.0,
            PdfChartKind::Pie | PdfChartKind::Doughnut => 184.0,
            PdfChartKind::Line | PdfChartKind::Area | PdfChartKind::Scatter => 224.0,
        }
    }

    fn pdf_draw_bar_chart(
        stream: &mut String,
        chart: &PdfChart,
        x: f64,
        top: f64,
        width: f64,
        height: f64,
        body_font: usize,
    ) {
        let rows = chart.rows.iter().take(10).collect::<Vec<_>>();
        let max = rows
            .iter()
            .filter_map(|row| row.values.first().copied())
            .map(f64::abs)
            .fold(1.0, f64::max);
        let label_x = x + 16.0;
        let bar_x = x + 158.0;
        let value_x = x + width - 62.0;
        let bar_width = (width - 244.0).max(120.0);
        let mut row_y = top - 58.0;
        if let Some(name) = chart.series_names.first() {
            Self::pdf_add_text(stream, bar_x, top - 39.0, 8, name);
        }
        for (index, row) in rows.iter().enumerate() {
            let value = row.values.first().copied().unwrap_or_default();
            let fill_width = (value.abs() / max * bar_width).max(2.0);
            Self::pdf_add_text(
                stream,
                label_x,
                row_y,
                body_font.saturating_sub(2),
                &Self::pdf_short_text(&row.label, 22),
            );
            Self::pdf_rect(
                stream,
                bar_x,
                row_y - 5.0,
                bar_width,
                8.0,
                (0.90, 0.93, 0.95),
            );
            Self::pdf_rect(
                stream,
                bar_x,
                row_y - 5.0,
                fill_width,
                8.0,
                Self::pdf_color(index),
            );
            Self::pdf_add_text(
                stream,
                value_x,
                row_y,
                body_font.saturating_sub(2),
                &Self::pdf_chart_value_label(value),
            );
            row_y -= 19.0;
            if row_y < top - height + 18.0 {
                break;
            }
        }
    }

    fn pdf_draw_share_chart(
        stream: &mut String,
        chart: &PdfChart,
        x: f64,
        top: f64,
        width: f64,
        body_font: usize,
    ) {
        let rows = chart.rows.iter().take(12).collect::<Vec<_>>();
        let total = rows
            .iter()
            .filter_map(|row| row.values.first().copied())
            .filter(|value| *value > 0.0)
            .sum::<f64>();
        if total <= 0.0 {
            Self::pdf_draw_bar_chart(stream, chart, x, top, width, 160.0, body_font);
            return;
        }
        let bar_x = x + 16.0;
        let bar_y = top - 66.0;
        let bar_width = width - 32.0;
        Self::pdf_rect(stream, bar_x, bar_y, bar_width, 15.0, (0.90, 0.93, 0.95));
        let mut cursor = bar_x;
        for (index, row) in rows.iter().enumerate() {
            let value = row.values.first().copied().unwrap_or_default().max(0.0);
            if value <= 0.0 {
                continue;
            }
            let segment_width = value / total * bar_width;
            Self::pdf_rect(
                stream,
                cursor,
                bar_y,
                segment_width.max(1.5),
                15.0,
                Self::pdf_color(index),
            );
            cursor += segment_width;
        }
        Self::pdf_add_text(
            stream,
            bar_x,
            bar_y - 18.0,
            body_font.saturating_sub(2),
            &format!("Total: {}", Self::pdf_chart_value_label(total)),
        );
        let mut legend_y = bar_y - 42.0;
        for (index, row) in rows.iter().enumerate() {
            let value = row.values.first().copied().unwrap_or_default();
            Self::pdf_rect(
                stream,
                bar_x,
                legend_y - 4.0,
                8.0,
                8.0,
                Self::pdf_color(index),
            );
            Self::pdf_add_text(
                stream,
                bar_x + 14.0,
                legend_y,
                body_font.saturating_sub(2),
                &Self::pdf_short_text(&row.label, 30),
            );
            Self::pdf_add_text(
                stream,
                x + width - 70.0,
                legend_y,
                body_font.saturating_sub(2),
                &Self::pdf_chart_value_label(value),
            );
            legend_y -= 16.0;
        }
    }

    fn pdf_draw_line_chart(
        stream: &mut String,
        chart: &PdfChart,
        x: f64,
        top: f64,
        width: f64,
        height: f64,
        body_font: usize,
    ) {
        let rows = chart.rows.iter().take(14).collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        let series_count = chart.series_names.len().clamp(1, 4);
        let values = rows
            .iter()
            .flat_map(|row| row.values.iter().take(series_count).copied())
            .collect::<Vec<_>>();
        let min = values.iter().copied().fold(0.0, f64::min);
        let max = values.iter().copied().fold(1.0, f64::max);
        let span = if (max - min).abs() < f64::EPSILON {
            1.0
        } else {
            max - min
        };
        let plot_x = x + 50.0;
        let plot_y = top - height + 44.0;
        let plot_width = width - 78.0;
        let plot_height = height - 104.0;
        Self::pdf_line(
            stream,
            plot_x,
            plot_y,
            plot_x + plot_width,
            plot_y,
            0.8,
            (0.55, 0.62, 0.66),
        );
        Self::pdf_line(
            stream,
            plot_x,
            plot_y,
            plot_x,
            plot_y + plot_height,
            0.8,
            (0.55, 0.62, 0.66),
        );
        Self::pdf_add_text(
            stream,
            x + 12.0,
            plot_y + plot_height - 2.0,
            body_font.saturating_sub(3),
            &Self::pdf_chart_value_label(max),
        );
        Self::pdf_add_text(
            stream,
            x + 12.0,
            plot_y - 2.0,
            body_font.saturating_sub(3),
            &Self::pdf_chart_value_label(min),
        );
        let x_for = |index: usize| -> f64 {
            if rows.len() <= 1 {
                plot_x + plot_width / 2.0
            } else {
                plot_x + index as f64 / (rows.len() - 1) as f64 * plot_width
            }
        };
        let y_for = |value: f64| -> f64 { plot_y + (value - min) / span * plot_height };
        for series_index in 0..series_count {
            let color = Self::pdf_color(series_index);
            let points = rows
                .iter()
                .enumerate()
                .filter_map(|(row_index, row)| {
                    row.values
                        .get(series_index)
                        .copied()
                        .map(|value| (x_for(row_index), y_for(value)))
                })
                .collect::<Vec<_>>();
            if points.is_empty() {
                continue;
            }
            let (r, g, b) = color;
            stream.push_str(&format!("{r:.3} {g:.3} {b:.3} RG\n1.6 w\n"));
            for (point_index, (px, py)) in points.iter().enumerate() {
                if point_index == 0 {
                    stream.push_str(&format!("{px:.1} {py:.1} m\n"));
                } else {
                    stream.push_str(&format!("{px:.1} {py:.1} l\n"));
                }
            }
            if chart.kind != PdfChartKind::Scatter {
                stream.push_str("S\n");
            }
            for (px, py) in points {
                Self::pdf_rect(stream, px - 2.0, py - 2.0, 4.0, 4.0, color);
            }
        }
        let label_step = rows.len().div_ceil(5).max(1);
        for (index, row) in rows.iter().enumerate() {
            if index % label_step != 0 && index + 1 != rows.len() {
                continue;
            }
            Self::pdf_add_text(
                stream,
                x_for(index) - 14.0,
                plot_y - 18.0,
                body_font.saturating_sub(3),
                &Self::pdf_short_text(&row.label, 10),
            );
        }
    }

    fn pdf_draw_chart(stream: &mut String, chart: &PdfChart, top: f64, body_font: usize) {
        let x = 68.0;
        let width = 476.0;
        let height = Self::pdf_chart_height(chart);
        let bottom = top - height;
        Self::pdf_rect(stream, x, bottom, width, height - 8.0, (0.97, 0.98, 0.98));
        Self::pdf_add_text(stream, x + 14.0, top - 24.0, body_font + 1, &chart.title);
        if !chart.subtitle.trim().is_empty() {
            Self::pdf_add_text(
                stream,
                x + 14.0,
                top - 39.0,
                body_font.saturating_sub(2),
                &chart.subtitle,
            );
        }
        match chart.kind {
            PdfChartKind::Pie | PdfChartKind::Doughnut => {
                Self::pdf_draw_share_chart(stream, chart, x, top, width, body_font);
            }
            PdfChartKind::Line | PdfChartKind::Area | PdfChartKind::Scatter => {
                Self::pdf_draw_line_chart(stream, chart, x, top, width, height, body_font);
            }
            PdfChartKind::Bar => {
                Self::pdf_draw_bar_chart(stream, chart, x, top, width, height, body_font);
            }
        }
    }

    pub(in crate::runtime) fn generate_simple_pdf_bytes(
        title: &str,
        content: &str,
        style: &str,
    ) -> Vec<u8> {
        const PAGE_WIDTH: usize = 612;
        const PAGE_HEIGHT: usize = 792;
        const TOP_Y: f64 = 740.0;
        const BOTTOM_Y: f64 = 56.0;
        const LINE_HEIGHT: f64 = 16.0;
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

        let mut page_streams = Vec::new();
        let mut stream = String::new();
        let mut y = TOP_Y;
        Self::pdf_add_text(&mut stream, 72.0, y, title_font, title.trim());
        y -= LINE_HEIGHT * 2.0;

        let mut render_elements = Self::pdf_auto_chart_elements(content);
        render_elements.extend(Self::pdf_render_elements(content, 92));

        for element in render_elements {
            match element {
                PdfRenderElement::Text(line) => {
                    if y < BOTTOM_Y {
                        page_streams.push(std::mem::take(&mut stream));
                        y = TOP_Y;
                    }
                    if !line.trim().is_empty() {
                        Self::pdf_add_text(&mut stream, 72.0, y, body_font, &line);
                    }
                    y -= LINE_HEIGHT;
                }
                PdfRenderElement::Chart(chart) => {
                    let chart_height = Self::pdf_chart_height(&chart);
                    if y - chart_height < BOTTOM_Y {
                        page_streams.push(std::mem::take(&mut stream));
                        y = TOP_Y;
                    }
                    Self::pdf_draw_chart(&mut stream, &chart, y, body_font);
                    y -= chart_height + 8.0;
                }
            }
        }
        page_streams.push(stream);

        let page_count = page_streams.len().max(1);
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
        for (index, stream) in page_streams.iter().enumerate() {
            let page_id = first_page_id + index * 2;
            let content_id = page_id + 1;
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
