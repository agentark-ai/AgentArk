//! Shared outbound message splitting for push notifications and chat replies.

pub const PUSH_NOTIFICATION_MAX_CHARS: usize = 1000;
pub const DEFAULT_PROVIDER_SAFE_MAX_CHARS: usize = 3500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitProfile {
    pub max_chars: usize,
    pub label_chunks: bool,
}

impl SplitProfile {
    pub const fn push_notification() -> Self {
        Self {
            max_chars: PUSH_NOTIFICATION_MAX_CHARS,
            label_chunks: true,
        }
    }

    pub const fn provider_safe(max_chars: usize) -> Self {
        Self {
            max_chars,
            label_chunks: false,
        }
    }
}

pub fn provider_safe_limit_for_channel(channel: &str) -> usize {
    match channel.trim().to_ascii_lowercase().as_str() {
        "telegram" => 4000,
        "whatsapp" => 4000,
        "discord" => 1900,
        "slack" => 39_000,
        "line" => 4000,
        "matrix" | "teams" | "google_chat" | "signal" | "imessage" | "wechat" | "qq" => {
            DEFAULT_PROVIDER_SAFE_MAX_CHARS
        }
        _ => DEFAULT_PROVIDER_SAFE_MAX_CHARS,
    }
}

pub fn split_for_push_notification(text: &str) -> Vec<String> {
    split_outbound_message(text, SplitProfile::push_notification())
}

pub fn split_for_provider_safe_channel(channel: &str, text: &str) -> Vec<String> {
    split_outbound_message(
        text,
        SplitProfile::provider_safe(provider_safe_limit_for_channel(channel)),
    )
}

pub fn split_outbound_message(text: &str, profile: SplitProfile) -> Vec<String> {
    let max_chars = profile.max_chars.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    if !profile.label_chunks {
        return split_text_to_char_limit(text, max_chars);
    }

    let mut total = 0usize;
    loop {
        let label_width = chunk_label_width(total.max(1), total.max(1));
        let body_limit = max_chars.saturating_sub(label_width).max(1);
        let chunks = split_text_to_char_limit(text, body_limit);
        if chunks.len() == total || total == 0 {
            total = chunks.len();
            let final_label_width = chunk_label_width(total.max(1), total.max(1));
            let final_body_limit = max_chars.saturating_sub(final_label_width).max(1);
            let final_chunks = split_text_to_char_limit(text, final_body_limit);
            if final_chunks.len() == total {
                return label_chunks(final_chunks);
            }
            total = final_chunks.len();
            continue;
        }
        total = chunks.len();
    }
}

fn label_chunks(chunks: Vec<String>) -> Vec<String> {
    let total = chunks.len();
    if total <= 1 {
        return chunks;
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk)| format!("[{}/{}] {}", idx + 1, total, chunk))
        .collect()
}

fn chunk_label_width(index: usize, total: usize) -> usize {
    format!("[{}/{}] ", index, total).chars().count()
}

fn split_text_to_char_limit(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.chars().count() <= max_chars {
            chunks.push(rest.to_string());
            break;
        }

        let split_at = best_split_byte(rest, max_chars);
        let (head, tail) = rest.split_at(split_at);
        let head = head.trim_end();
        if head.is_empty() {
            let hard_at = byte_index_after_chars(rest, max_chars).unwrap_or(rest.len());
            let (head, tail) = rest.split_at(hard_at);
            chunks.push(head.to_string());
            rest = tail.trim_start();
        } else {
            chunks.push(head.to_string());
            rest = tail.trim_start();
        }
    }

    chunks
}

fn best_split_byte(text: &str, max_chars: usize) -> usize {
    let hard_limit = byte_index_after_chars(text, max_chars).unwrap_or(text.len());
    let window = &text[..hard_limit];
    let min_chars = (max_chars * 3 / 5).max(1);
    let min_byte = byte_index_after_chars(text, min_chars)
        .unwrap_or(0)
        .min(hard_limit);

    for boundary in ["\n\n", "\n"] {
        if let Some(idx) = window.rfind(boundary) {
            if idx >= min_byte {
                return idx;
            }
        }
    }

    let mut last_whitespace = None;
    for (idx, ch) in window.char_indices() {
        if ch.is_whitespace() && idx >= min_byte {
            last_whitespace = Some(idx);
        }
    }
    last_whitespace.unwrap_or(hard_limit)
}

fn byte_index_after_chars(text: &str, max_chars: usize) -> Option<usize> {
    if max_chars == 0 {
        return Some(0);
    }
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => Some(idx),
        None => {
            if text.chars().count() <= max_chars {
                Some(text.len())
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_push_label(chunk: &str) -> &str {
        let Some(end) = chunk.find("] ") else {
            return chunk;
        };
        &chunk[end + 2..]
    }

    #[test]
    fn provider_split_preserves_unicode_boundaries() {
        let smile = char::from_u32(0x1F642).expect("valid emoji scalar");
        let text = format!("{}{}{}", "a".repeat(9), smile, "b".repeat(9));
        let chunks = split_outbound_message(&text, SplitProfile::provider_safe(10));

        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 10));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn provider_split_prefers_paragraph_boundaries() {
        let text = "first paragraph stays together\n\nsecond paragraph stays together";
        let chunks = split_outbound_message(text, SplitProfile::provider_safe(38));

        assert_eq!(
            chunks,
            vec![
                "first paragraph stays together",
                "second paragraph stays together"
            ]
        );
    }

    #[test]
    fn provider_split_prefers_line_boundaries() {
        let text = "line one has enough room\nline two has enough room";
        let chunks = split_outbound_message(text, SplitProfile::provider_safe(30));

        assert_eq!(
            chunks,
            vec!["line one has enough room", "line two has enough room"]
        );
    }

    #[test]
    fn provider_split_prefers_word_boundaries() {
        let text = "alpha beta gamma delta epsilon";
        let chunks = split_outbound_message(text, SplitProfile::provider_safe(18));

        assert_eq!(chunks, vec!["alpha beta gamma", "delta epsilon"]);
    }

    #[test]
    fn provider_split_hard_cuts_long_single_lines() {
        let text = "x".repeat(25);
        let chunks = split_outbound_message(&text, SplitProfile::provider_safe(10));

        assert_eq!(chunks, vec!["x".repeat(10), "x".repeat(10), "x".repeat(5)]);
    }

    #[test]
    fn push_split_labels_every_chunk_under_limit() {
        let text = "x ".repeat(1400);
        let chunks = split_for_push_notification(&text);

        assert!(chunks.len() > 2);
        assert!(chunks[0].starts_with("[1/"));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= PUSH_NOTIFICATION_MAX_CHARS));
    }

    #[test]
    fn push_split_does_not_label_single_chunk() {
        let text = "Short notification.";
        let chunks = split_for_push_notification(text);

        assert_eq!(chunks, vec![text.to_string()]);
    }

    #[test]
    fn push_split_sends_all_chunks_for_huge_messages() {
        let text = "z".repeat(PUSH_NOTIFICATION_MAX_CHARS * 4 + 123);
        let chunks = split_for_push_notification(&text);
        let joined = chunks
            .iter()
            .map(|chunk| strip_push_label(chunk))
            .collect::<String>();

        assert_eq!(joined, text);
        assert!(chunks.len() >= 5);
    }
}
