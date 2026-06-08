use serde_json::Value;

pub const MEMORY_CATEGORY_PROFILE_FACT: &str = "profile_fact";
pub const MEMORY_CATEGORY_ASSISTANT_PREFERENCE: &str = "assistant_preference";
pub const MEMORY_CATEGORY_WORK_PREFERENCE: &str = "work_preference";
pub const MEMORY_CATEGORY_PROJECT_DOMAIN: &str = "project_domain_memory";
pub const MEMORY_CATEGORY_EPHEMERAL_CONTEXT: &str = "ephemeral_context";
pub const MEMORY_CATEGORY_KNOWLEDGE: &str = "knowledge";
pub const MEMORY_CATEGORY_OTHER: &str = "other";

pub const MEMORY_CATEGORIES: &[&str] = &[
    MEMORY_CATEGORY_PROFILE_FACT,
    MEMORY_CATEGORY_ASSISTANT_PREFERENCE,
    MEMORY_CATEGORY_WORK_PREFERENCE,
    MEMORY_CATEGORY_PROJECT_DOMAIN,
    MEMORY_CATEGORY_EPHEMERAL_CONTEXT,
    MEMORY_CATEGORY_KNOWLEDGE,
    MEMORY_CATEGORY_OTHER,
];

pub fn normalize_memory_category(
    raw_category: Option<&str>,
    semantic_kind: Option<&str>,
) -> &'static str {
    let category = raw_category.unwrap_or_default().trim();
    if MEMORY_CATEGORIES.contains(&category) {
        return MEMORY_CATEGORIES
            .iter()
            .copied()
            .find(|known| *known == category)
            .unwrap_or(MEMORY_CATEGORY_OTHER);
    }

    match semantic_kind.unwrap_or_default().trim() {
        "identity" | "location" | "timezone" | "contact" | "relationship" | "personal_fact" => {
            MEMORY_CATEGORY_PROFILE_FACT
        }
        "assistant_preference" => MEMORY_CATEGORY_ASSISTANT_PREFERENCE,
        "preference" | "workflow" | "constraint" | "work_preference" => {
            MEMORY_CATEGORY_WORK_PREFERENCE
        }
        "project_domain_memory" | "domain_memory" => MEMORY_CATEGORY_PROJECT_DOMAIN,
        "ephemeral_context" => MEMORY_CATEGORY_EPHEMERAL_CONTEXT,
        "knowledge" => MEMORY_CATEGORY_KNOWLEDGE,
        _ => MEMORY_CATEGORY_OTHER,
    }
}

pub fn normalize_memory_slot_key(raw: &str, max_len: usize) -> Option<String> {
    let key = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if key.is_empty() || key.len() > max_len {
        return None;
    }
    Some(key)
}

fn normalize_memory_keyish_text(raw: &str) -> Option<String> {
    let key = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if key.is_empty() {
        return None;
    }
    Some(key)
}

fn memory_text_tokens_for_key_repair(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut normalized = String::new();
    let mut original = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            original.push(ch);
        } else if !normalized.is_empty() {
            out.push((normalized.clone(), original.clone()));
            normalized.clear();
            original.clear();
        }
    }
    if !normalized.is_empty() {
        out.push((normalized, original));
    }
    out
}

fn memory_value_suffix_from_key_suffix(raw_value: &str, key_suffix: &[&str]) -> Option<String> {
    if key_suffix.is_empty() {
        return None;
    }
    let value_tokens = memory_text_tokens_for_key_repair(raw_value);
    if value_tokens.len() < key_suffix.len() {
        return None;
    }
    let value_suffix = &value_tokens[value_tokens.len() - key_suffix.len()..];
    if value_suffix
        .iter()
        .zip(key_suffix.iter())
        .all(|((normalized, _), key_segment)| normalized == *key_segment)
    {
        let repaired = value_suffix
            .iter()
            .map(|(_, original)| original.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let value = repaired
            .trim()
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'))
            .to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

pub fn repair_memory_slot_key_and_value(
    raw_key: &str,
    raw_value: &str,
    allow_value_suffix_repair: bool,
) -> Option<(String, Option<String>)> {
    let key = normalize_memory_slot_key(raw_key, 80)?;
    let value_key = normalize_memory_keyish_text(raw_value)?;
    if key == value_key {
        return None;
    }

    if !allow_value_suffix_repair {
        return Some((key, None));
    }

    let key_segments = key
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let value_segments = value_key
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if key_segments.len() <= 1 || value_segments.is_empty() {
        return Some((key, None));
    }

    let max_suffix_len = key_segments.len().min(value_segments.len() + 1) - 1;
    for suffix_len in (1..=max_suffix_len).rev() {
        let key_suffix = &key_segments[key_segments.len() - suffix_len..];
        let value_suffix = &value_segments[value_segments.len() - suffix_len..];
        if key_suffix == value_suffix {
            let repaired = key_segments[..key_segments.len() - suffix_len].join("_");
            let repaired_key = normalize_memory_slot_key(&repaired, 80)?;
            let repaired_value = memory_value_suffix_from_key_suffix(raw_value, key_suffix);
            return Some((repaired_key, repaired_value));
        }
    }

    Some((key, None))
}

pub fn memory_category_from_metadata(
    metadata: &Value,
    semantic_kind: Option<&str>,
) -> &'static str {
    let raw_category = metadata
        .get("memory_category")
        .and_then(|value| value.as_str());
    normalize_memory_category(raw_category, semantic_kind)
}

pub fn memory_category_label(category: &str) -> &'static str {
    match category {
        MEMORY_CATEGORY_PROFILE_FACT => "Profile fact",
        MEMORY_CATEGORY_ASSISTANT_PREFERENCE => "Assistant preference",
        MEMORY_CATEGORY_WORK_PREFERENCE => "Work preference",
        MEMORY_CATEGORY_PROJECT_DOMAIN => "Project/domain memory",
        MEMORY_CATEGORY_EPHEMERAL_CONTEXT => "Ephemeral context",
        MEMORY_CATEGORY_KNOWLEDGE => "Knowledge",
        _ => "Other memory",
    }
}

pub fn memory_category_prompt_cap(category: &str) -> usize {
    match category {
        MEMORY_CATEGORY_PROFILE_FACT => 3,
        MEMORY_CATEGORY_ASSISTANT_PREFERENCE => 2,
        MEMORY_CATEGORY_WORK_PREFERENCE => 2,
        MEMORY_CATEGORY_PROJECT_DOMAIN => 3,
        MEMORY_CATEGORY_EPHEMERAL_CONTEXT => 2,
        MEMORY_CATEGORY_KNOWLEDGE => 1,
        _ => 1,
    }
}

pub fn memory_category_requires_topical_relevance(category: &str) -> bool {
    matches!(
        category,
        MEMORY_CATEGORY_WORK_PREFERENCE
            | MEMORY_CATEGORY_PROJECT_DOMAIN
            | MEMORY_CATEGORY_KNOWLEDGE
            | MEMORY_CATEGORY_EPHEMERAL_CONTEXT
    )
}

pub fn memory_category_is_ephemeral(category: &str) -> bool {
    category == MEMORY_CATEGORY_EPHEMERAL_CONTEXT
}

pub fn normalize_memory_topics(value: Option<&Value>, max_topics: usize) -> Vec<String> {
    let mut topics = Vec::new();
    let Some(value) = value else {
        return topics;
    };
    let values: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::String(_) => vec![value],
        _ => Vec::new(),
    };
    for item in values {
        let Some(topic) = item
            .as_str()
            .map(str::trim)
            .filter(|topic| !topic.is_empty())
        else {
            continue;
        };
        let normalized = topic
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | ' ') {
                    ch
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .to_ascii_lowercase();
        if normalized.is_empty() || topics.iter().any(|existing| existing == &normalized) {
            continue;
        }
        topics.push(normalized);
        if topics.len() >= max_topics {
            break;
        }
    }
    topics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_schema_category_wins_over_kind_fallback() {
        assert_eq!(
            normalize_memory_category(Some(MEMORY_CATEGORY_PROJECT_DOMAIN), Some("preference")),
            MEMORY_CATEGORY_PROJECT_DOMAIN
        );
    }

    #[test]
    fn canonical_preference_kind_defaults_to_work_preference() {
        assert_eq!(
            normalize_memory_category(None, Some("preference")),
            MEMORY_CATEGORY_WORK_PREFERENCE
        );
    }

    #[test]
    fn topics_are_normalized_and_bounded() {
        let topics = normalize_memory_topics(
            Some(&serde_json::json!([
                "Financial Modeling",
                "META/equity",
                "Financial Modeling"
            ])),
            4,
        );
        assert_eq!(topics, vec!["financial_modeling", "meta/equity"]);
    }
}
