use anyhow::{Result, anyhow};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamBlockEvent {
    Text(String),
    FileStart {
        path: String,
    },
    FileDelta {
        path: String,
        delta: String,
        snapshot: String,
    },
    FileEnd {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Checklist {
        items: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamBlockState {
    Text,
    File {
        path: String,
        encoding: Option<String>,
        raw_open_tag: String,
        start_emitted: bool,
        content: String,
    },
    Checklist {
        raw_open_tag: String,
        content: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct StreamBlockParser {
    state: StreamBlockState,
    buffer: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ParsedStreamBlocks {
    pub files: BTreeMap<String, String>,
    pub delete_paths: Vec<String>,
    pub delete_orphans: bool,
    pub checklist_items: Vec<String>,
}

impl ParsedStreamBlocks {
    pub(crate) fn has_operations(&self) -> bool {
        !self.files.is_empty() || !self.delete_paths.is_empty() || self.delete_orphans
    }
}

pub(crate) fn parse_stream_blocks_from_text(text: &str) -> ParsedStreamBlocks {
    let mut parser = StreamBlockParser::new();
    let mut events = parser.feed(text);
    events.extend(parser.finish());
    let mut parsed = ParsedStreamBlocks::default();
    for event in events {
        match event {
            StreamBlockEvent::FileEnd { path, content } => {
                parsed.files.insert(path, content);
            }
            StreamBlockEvent::Delete { path } => {
                if path == "*" {
                    parsed.delete_orphans = true;
                } else if !parsed.delete_paths.iter().any(|existing| existing == &path) {
                    parsed.delete_paths.push(path);
                }
            }
            StreamBlockEvent::Checklist { items } => {
                for item in items {
                    if !parsed
                        .checklist_items
                        .iter()
                        .any(|existing| existing == &item)
                    {
                        parsed.checklist_items.push(item);
                    }
                }
            }
            StreamBlockEvent::Text(_)
            | StreamBlockEvent::FileStart { .. }
            | StreamBlockEvent::FileDelta { .. } => {}
        }
    }
    parsed
}

impl Default for StreamBlockParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamBlockParser {
    pub(crate) fn new() -> Self {
        Self {
            state: StreamBlockState::Text,
            buffer: String::new(),
        }
    }

    pub(crate) fn feed(&mut self, chunk: &str) -> Vec<StreamBlockEvent> {
        self.buffer.push_str(chunk);
        self.drain(false)
    }

    pub(crate) fn finish(&mut self) -> Vec<StreamBlockEvent> {
        self.drain(true)
    }

    fn drain(&mut self, finish: bool) -> Vec<StreamBlockEvent> {
        let mut events = Vec::new();
        loop {
            match &mut self.state {
                StreamBlockState::Text => {
                    let Some(next) = next_block_start(&self.buffer) else {
                        let emit_len = if finish {
                            self.buffer.len()
                        } else {
                            safe_text_emit_len(&self.buffer)
                        };
                        if emit_len > 0 {
                            let text = self.buffer[..emit_len].to_string();
                            self.buffer.drain(..emit_len);
                            events.push(StreamBlockEvent::Text(text));
                            continue;
                        }
                        break;
                    };
                    if next > 0 {
                        let text = self.buffer[..next].to_string();
                        self.buffer.drain(..next);
                        events.push(StreamBlockEvent::Text(text));
                        continue;
                    }
                    let Some(tag_end) = self.buffer.find('>') else {
                        break;
                    };
                    let raw_tag = self.buffer[..=tag_end].to_string();
                    if raw_tag.starts_with("<file") {
                        match parse_opening_tag(&raw_tag, "file") {
                            Ok(attrs) => {
                                let path = match attrs.get("path") {
                                    Some(raw) => match normalize_block_path(raw, false) {
                                        Ok(path) => path,
                                        Err(_) => {
                                            let text = self.buffer[..=tag_end].to_string();
                                            self.buffer.drain(..=tag_end);
                                            events.push(StreamBlockEvent::Text(text));
                                            continue;
                                        }
                                    },
                                    None => {
                                        let text = self.buffer[..=tag_end].to_string();
                                        self.buffer.drain(..=tag_end);
                                        events.push(StreamBlockEvent::Text(text));
                                        continue;
                                    }
                                };
                                let encoding = attrs
                                    .get("encoding")
                                    .map(|value| value.trim().to_ascii_lowercase())
                                    .filter(|value| !value.is_empty());
                                if encoding.as_deref().is_some_and(|value| value != "base64") {
                                    let text = self.buffer[..=tag_end].to_string();
                                    self.buffer.drain(..=tag_end);
                                    events.push(StreamBlockEvent::Text(text));
                                    continue;
                                }
                                let emit_start = encoding.as_deref() != Some("base64");
                                self.buffer.drain(..=tag_end);
                                self.state = StreamBlockState::File {
                                    path: path.clone(),
                                    encoding,
                                    raw_open_tag: raw_tag,
                                    start_emitted: emit_start,
                                    content: String::new(),
                                };
                                if emit_start {
                                    events.push(StreamBlockEvent::FileStart { path });
                                }
                                continue;
                            }
                            Err(_) => {
                                let text = self.buffer[..=tag_end].to_string();
                                self.buffer.drain(..=tag_end);
                                events.push(StreamBlockEvent::Text(text));
                                continue;
                            }
                        }
                    }
                    if raw_tag.starts_with("<delete") {
                        match parse_opening_tag(&raw_tag, "delete").and_then(|attrs| {
                            attrs
                                .get("path")
                                .ok_or_else(|| anyhow!("delete block requires path"))
                                .and_then(|raw| normalize_block_path(raw, true))
                        }) {
                            Ok(path) => {
                                self.buffer.drain(..=tag_end);
                                if let Some(close) = self.buffer.find("</delete>") {
                                    self.buffer.drain(..close + "</delete>".len());
                                }
                                events.push(StreamBlockEvent::Delete { path });
                                continue;
                            }
                            Err(_) => {
                                let text = self.buffer[..=tag_end].to_string();
                                self.buffer.drain(..=tag_end);
                                events.push(StreamBlockEvent::Text(text));
                                continue;
                            }
                        }
                    }
                    if raw_tag.starts_with("<checklist") {
                        match parse_opening_tag(&raw_tag, "checklist") {
                            Ok(_) => {
                                self.buffer.drain(..=tag_end);
                                self.state = StreamBlockState::Checklist {
                                    raw_open_tag: raw_tag,
                                    content: String::new(),
                                };
                                continue;
                            }
                            Err(_) => {
                                let text = self.buffer[..=tag_end].to_string();
                                self.buffer.drain(..=tag_end);
                                events.push(StreamBlockEvent::Text(text));
                                continue;
                            }
                        }
                    }
                    let emit_len = if finish {
                        self.buffer.len()
                    } else {
                        safe_text_emit_len(&self.buffer)
                    };
                    if emit_len > 0 {
                        let text = self.buffer[..emit_len].to_string();
                        self.buffer.drain(..emit_len);
                        events.push(StreamBlockEvent::Text(text));
                        continue;
                    }
                    break;
                }
                StreamBlockState::File {
                    path,
                    encoding,
                    raw_open_tag,
                    start_emitted,
                    content,
                } => {
                    let is_base64 = encoding.as_deref() == Some("base64");
                    let Some(close_start) = self.buffer.find("</file>") else {
                        if is_base64 {
                            break;
                        }
                        let emit_len = if finish {
                            self.buffer.len()
                        } else {
                            safe_file_body_emit_len(&self.buffer)
                        };
                        if emit_len > 0 {
                            let delta = self.buffer[..emit_len].to_string();
                            self.buffer.drain(..emit_len);
                            content.push_str(&delta);
                            events.push(StreamBlockEvent::FileDelta {
                                path: path.clone(),
                                delta,
                                snapshot: content.clone(),
                            });
                            continue;
                        }
                        break;
                    };
                    let raw_body = self.buffer[..close_start].to_string();
                    self.buffer.drain(..close_start + "</file>".len());
                    let decoded = if is_base64 {
                        match decode_base64_text(&raw_body) {
                            Ok(decoded) => decoded,
                            Err(_) => {
                                events.push(StreamBlockEvent::Text(format!(
                                    "{}{}</file>",
                                    raw_open_tag.as_str(),
                                    raw_body
                                )));
                                self.state = StreamBlockState::Text;
                                continue;
                            }
                        }
                    } else {
                        raw_body
                    };
                    if !*start_emitted {
                        events.push(StreamBlockEvent::FileStart { path: path.clone() });
                        *start_emitted = true;
                    }
                    if !decoded.is_empty() {
                        content.push_str(&decoded);
                        events.push(StreamBlockEvent::FileDelta {
                            path: path.clone(),
                            delta: decoded,
                            snapshot: content.clone(),
                        });
                    }
                    events.push(StreamBlockEvent::FileEnd {
                        path: path.clone(),
                        content: content.clone(),
                    });
                    self.state = StreamBlockState::Text;
                    continue;
                }
                StreamBlockState::Checklist {
                    raw_open_tag,
                    content,
                } => {
                    let Some(close_start) = self.buffer.find("</checklist>") else {
                        if finish {
                            content.push_str(&self.buffer);
                            self.buffer.clear();
                            events.push(StreamBlockEvent::Text(format!(
                                "{}{}",
                                raw_open_tag.as_str(),
                                content
                            )));
                            self.state = StreamBlockState::Text;
                            continue;
                        }
                        let emit_len = if finish {
                            self.buffer.len()
                        } else {
                            safe_checklist_body_emit_len(&self.buffer)
                        };
                        if emit_len > 0 {
                            let delta = self.buffer[..emit_len].to_string();
                            self.buffer.drain(..emit_len);
                            content.push_str(&delta);
                            continue;
                        }
                        break;
                    };
                    let raw_body = self.buffer[..close_start].to_string();
                    self.buffer.drain(..close_start + "</checklist>".len());
                    content.push_str(&raw_body);
                    let items = parse_checklist_items(content);
                    if items.is_empty() {
                        events.push(StreamBlockEvent::Text(format!(
                            "{}{}</checklist>",
                            raw_open_tag.as_str(),
                            content
                        )));
                    } else {
                        events.push(StreamBlockEvent::Checklist { items });
                    }
                    self.state = StreamBlockState::Text;
                    continue;
                }
            }
        }
        events
    }
}

fn next_block_start(buffer: &str) -> Option<usize> {
    ["<file", "<delete", "<checklist"]
        .iter()
        .filter_map(|needle| buffer.find(needle))
        .min()
}

fn safe_text_emit_len(buffer: &str) -> usize {
    let mut keep = 0usize;
    for prefix in ["<file", "<delete", "<checklist"] {
        let max = buffer.len().min(prefix.len().saturating_sub(1));
        for len in 1..=max {
            if buffer.ends_with(&prefix[..len]) {
                keep = keep.max(len);
            }
        }
    }
    buffer.len().saturating_sub(keep)
}

fn safe_checklist_body_emit_len(buffer: &str) -> usize {
    let close = "</checklist>";
    let mut keep = 0usize;
    let max = buffer.len().min(close.len().saturating_sub(1));
    for len in 1..=max {
        if buffer.ends_with(&close[..len]) {
            keep = keep.max(len);
        }
    }
    buffer.len().saturating_sub(keep)
}

fn parse_checklist_items(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = raw;
    while let Some(open_idx) = rest.find("<item>") {
        rest = &rest[open_idx + "<item>".len()..];
        let Some(close_idx) = rest.find("</item>") else {
            break;
        };
        let item = decode_basic_xml_entities(rest[..close_idx].trim());
        if !item.is_empty() && !items.iter().any(|existing| existing == &item) {
            items.push(item);
        }
        rest = &rest[close_idx + "</item>".len()..];
    }
    items
}

fn decode_basic_xml_entities(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn safe_file_body_emit_len(buffer: &str) -> usize {
    let close = "</file>";
    let mut keep = 0usize;
    let max = buffer.len().min(close.len().saturating_sub(1));
    for len in 1..=max {
        if buffer.ends_with(&close[..len]) {
            keep = keep.max(len);
        }
    }
    buffer.len().saturating_sub(keep)
}

fn parse_opening_tag(
    raw_tag: &str,
    expected: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let trimmed = raw_tag.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .ok_or_else(|| anyhow!("invalid block tag"))?
        .trim()
        .trim_end_matches('/')
        .trim();
    let rest = inner
        .strip_prefix(expected)
        .ok_or_else(|| anyhow!("unexpected block tag"))?;
    if !rest.is_empty() && !rest.chars().next().is_some_and(|ch| ch.is_whitespace()) {
        return Err(anyhow!("unexpected block tag"));
    }
    parse_attrs(rest)
}

fn parse_attrs(input: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut attrs = std::collections::HashMap::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
        {
            i += 1;
        }
        if i == key_start {
            return Err(anyhow!("invalid attribute key"));
        }
        let key = &input[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            return Err(anyhow!("attribute requires value"));
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            return Err(anyhow!("attribute value must be quoted"));
        }
        let quote = bytes[i];
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(anyhow!("unterminated attribute value"));
        }
        attrs.insert(key.to_string(), input[value_start..i].to_string());
        i += 1;
    }
    Ok(attrs)
}

fn normalize_block_path(raw: &str, allow_star: bool) -> Result<String> {
    let normalized = raw.trim().replace('\\', "/");
    if allow_star && normalized == "*" {
        return Ok(normalized);
    }
    if normalized.is_empty() {
        return Err(anyhow!("path is empty"));
    }
    let path = std::path::Path::new(&normalized);
    if path.is_absolute() {
        return Err(anyhow!("path must be app-relative"));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => return Err(anyhow!("path must be app-relative")),
        }
    }
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == ".")
    {
        return Err(anyhow!("path has an empty segment"));
    }
    if normalized == ".app_meta.json"
        || normalized == ".agentark_runtime_env"
        || normalized == ".agentark_runtime_stdout.log"
        || normalized == ".agentark_runtime_stderr.log"
        || normalized
            .split('/')
            .next()
            .is_some_and(|part| part == ".agentark" || part == ".git")
    {
        return Err(anyhow!("path targets internal app metadata"));
    }
    Ok(normalized)
}

fn decode_base64_text(raw: &str) -> Result<String> {
    use base64::Engine as _;
    let compact = raw.split_whitespace().collect::<String>();
    let bytes = base64::engine::general_purpose::STANDARD.decode(compact)?;
    Ok(String::from_utf8(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_streams_file_block_across_chunks() {
        let mut parser = StreamBlockParser::new();
        let mut events = Vec::new();
        events.extend(parser.feed("Before <fi"));
        events.extend(parser.feed("le path=\"index.html\">he"));
        events.extend(parser.feed("llo</file> after"));
        events.extend(parser.finish());

        assert_eq!(
            events,
            vec![
                StreamBlockEvent::Text("Before ".to_string()),
                StreamBlockEvent::FileStart {
                    path: "index.html".to_string()
                },
                StreamBlockEvent::FileDelta {
                    path: "index.html".to_string(),
                    delta: "he".to_string(),
                    snapshot: "he".to_string()
                },
                StreamBlockEvent::FileDelta {
                    path: "index.html".to_string(),
                    delta: "llo".to_string(),
                    snapshot: "hello".to_string()
                },
                StreamBlockEvent::FileEnd {
                    path: "index.html".to_string(),
                    content: "hello".to_string()
                },
                StreamBlockEvent::Text(" after".to_string()),
            ]
        );
    }

    #[test]
    fn parser_decodes_base64_file_body() {
        let mut parser = StreamBlockParser::new();
        let mut events =
            parser.feed("<file path=\"app.js\" encoding=\"base64\">Y29uc29sZS5sb2coMSk7</file>");
        events.extend(parser.finish());
        assert!(events.contains(&StreamBlockEvent::FileEnd {
            path: "app.js".to_string(),
            content: "console.log(1);".to_string(),
        }));
    }

    #[test]
    fn parser_rejects_invalid_base64_file_body_as_text() {
        let mut parser = StreamBlockParser::new();
        let mut events = parser.feed("<file path=\"app.js\" encoding=\"base64\">not valid!</file>");
        events.extend(parser.finish());

        assert_eq!(
            events,
            vec![StreamBlockEvent::Text(
                "<file path=\"app.js\" encoding=\"base64\">not valid!</file>".to_string()
            )]
        );
    }

    #[test]
    fn parser_rejects_unsafe_file_path_as_text() {
        let mut parser = StreamBlockParser::new();
        let mut events = parser.feed("<file path=\"../x\">bad</file>");
        events.extend(parser.finish());
        assert!(
            matches!(events.first(), Some(StreamBlockEvent::Text(text)) if text.contains("<file"))
        );
    }

    #[test]
    fn parser_emits_delete_event() {
        let mut parser = StreamBlockParser::new();
        let mut events = parser.feed("<delete path=\"old.css\"/>");
        events.extend(parser.finish());
        assert_eq!(
            events,
            vec![StreamBlockEvent::Delete {
                path: "old.css".to_string()
            }]
        );
    }

    #[test]
    fn parser_emits_checklist_event_across_chunks() {
        let mut parser = StreamBlockParser::new();
        let mut events = Vec::new();
        events.extend(parser.feed("before <check"));
        events.extend(parser.feed("list><item>Fetch arXiv data</item>"));
        events.extend(parser.feed("<item>Render filters &amp; counts</item></checklist> after"));
        events.extend(parser.finish());

        assert_eq!(
            events,
            vec![
                StreamBlockEvent::Text("before ".to_string()),
                StreamBlockEvent::Checklist {
                    items: vec![
                        "Fetch arXiv data".to_string(),
                        "Render filters & counts".to_string()
                    ]
                },
                StreamBlockEvent::Text(" after".to_string()),
            ]
        );
    }

    #[test]
    fn parse_stream_blocks_collects_files_deletes_and_checklist() {
        let parsed = parse_stream_blocks_from_text(
            "Note\n<file path=\"index.html\">hi</file><delete path=\"old.css\"/><delete path=\"*\"/><checklist><item>One</item></checklist>",
        );

        assert_eq!(parsed.files.get("index.html"), Some(&"hi".to_string()));
        assert_eq!(parsed.delete_paths, vec!["old.css".to_string()]);
        assert!(parsed.delete_orphans);
        assert_eq!(parsed.checklist_items, vec!["One".to_string()]);
    }
}
