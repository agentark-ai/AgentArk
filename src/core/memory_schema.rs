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
