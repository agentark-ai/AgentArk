use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResourceScopeClass {
    BrowserSession,
    FilePath,
    App,
    Watcher,
    IntegrationAccount,
    Repository,
    DatabaseTable,
    Conversation,
    ExternalResource,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ResourceScope {
    pub class: ResourceScopeClass,
    pub key: String,
}

impl ResourceScope {
    pub(super) fn new(class: ResourceScopeClass, key: impl Into<String>) -> Self {
        Self {
            class,
            key: safe_truncate(key.into().trim(), 220),
        }
    }

    pub(super) fn stable_key(&self) -> String {
        format!("{:?}:{}", &self.class, normalize_scope_atom(&self.key))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ActionResourceContract {
    pub action_name: String,
    #[serde(default)]
    pub read_scopes: Vec<ResourceScope>,
    #[serde(default)]
    pub write_scopes: Vec<ResourceScope>,
    #[serde(default)]
    pub exclusive_scopes: Vec<ResourceScope>,
    #[serde(default)]
    pub parallel_read_safe: bool,
}

impl ActionResourceContract {
    pub(super) fn all_scope_keys(&self) -> BTreeSet<String> {
        self.read_scopes
            .iter()
            .chain(self.write_scopes.iter())
            .chain(self.exclusive_scopes.iter())
            .map(ResourceScope::stable_key)
            .collect()
    }
}

pub(super) fn resource_contract_for_action(
    action: &crate::actions::ActionDef,
    arguments: Option<&serde_json::Value>,
) -> ActionResourceContract {
    let metadata = action.action_metadata();
    let mut read_scopes = BTreeSet::new();
    let mut write_scopes = BTreeSet::new();
    let mut exclusive_scopes = BTreeSet::new();

    for integration_id in &action.authorization.access.integration_ids {
        read_scopes.insert(ResourceScope::new(
            ResourceScopeClass::IntegrationAccount,
            integration_id.as_str(),
        ));
    }
    for target in &action.authorization.access.channel_targets {
        read_scopes.insert(ResourceScope::new(
            ResourceScopeClass::IntegrationAccount,
            format!("channel:{}", target.default_target),
        ));
    }

    let class_scope = integration_class_scope(&metadata.integration_class);
    match metadata.side_effect_level {
        crate::actions::ActionSideEffectLevel::None => {
            read_scopes.insert(ResourceScope::new(class_scope, action.name.as_str()));
        }
        crate::actions::ActionSideEffectLevel::Notify => {
            write_scopes.insert(ResourceScope::new(
                ResourceScopeClass::IntegrationAccount,
                format!("notify:{}", action.name),
            ));
        }
        crate::actions::ActionSideEffectLevel::Write => {
            write_scopes.insert(ResourceScope::new(class_scope.clone(), action.name.as_str()));
            if matches!(
                metadata.integration_class,
                crate::actions::ActionIntegrationClass::Browser
                    | crate::actions::ActionIntegrationClass::App
                    | crate::actions::ActionIntegrationClass::Filesystem
                    | crate::actions::ActionIntegrationClass::Code
            ) {
                exclusive_scopes.insert(ResourceScope::new(class_scope, action.name.as_str()));
            }
        }
    }

    if let Some(arguments) = arguments {
        collect_scopes_from_value(
            None,
            arguments,
            &mut read_scopes,
            &mut write_scopes,
            &mut exclusive_scopes,
            !matches!(
                metadata.side_effect_level,
                crate::actions::ActionSideEffectLevel::None
            ),
            0,
        );
    }

    let parallel_read_safe = action_is_parallel_read_safe(action)
        && write_scopes.is_empty()
        && exclusive_scopes.is_empty();

    ActionResourceContract {
        action_name: action.name.clone(),
        read_scopes: read_scopes.into_iter().collect(),
        write_scopes: write_scopes.into_iter().collect(),
        exclusive_scopes: exclusive_scopes.into_iter().collect(),
        parallel_read_safe,
    }
}

pub(super) fn tool_calls_are_parallel_safe(
    calls: &[crate::core::llm::ToolCall],
    action_by_name: &HashMap<String, crate::actions::ActionDef>,
) -> bool {
    if calls.len() <= 1 {
        return false;
    }

    let mut occupied = BTreeSet::new();
    for call in calls {
        let Some(action) = action_by_name.get(&call.name) else {
            return false;
        };
        let contract = resource_contract_for_action(action, Some(&call.arguments));
        if !contract.parallel_read_safe {
            return false;
        }
        for scope in contract.all_scope_keys() {
            if !occupied.insert(scope) {
                return false;
            }
        }
    }
    true
}

pub(super) fn action_is_parallel_read_safe(action: &crate::actions::ActionDef) -> bool {
    let metadata = action.action_metadata();
    matches!(
        metadata.side_effect_level,
        crate::actions::ActionSideEffectLevel::None
    ) && matches!(
        metadata.delivery_mode,
        crate::actions::ActionDeliveryMode::Immediate
    ) && matches!(
        metadata.role,
        crate::actions::ActionRole::DataSource
            | crate::actions::ActionRole::Inspection
            | crate::actions::ActionRole::Trigger
    ) && !matches!(
        metadata.integration_class,
        crate::actions::ActionIntegrationClass::Browser
            | crate::actions::ActionIntegrationClass::App
            | crate::actions::ActionIntegrationClass::Code
            | crate::actions::ActionIntegrationClass::Media
            | crate::actions::ActionIntegrationClass::Commerce
    ) && !action.authorization.human_approval.required
        && !action.authorization.outbound.outbound_write
        && !action.authorization.outbound.public_publish
}

fn integration_class_scope(
    class: &crate::actions::ActionIntegrationClass,
) -> ResourceScopeClass {
    match class {
        crate::actions::ActionIntegrationClass::Browser => ResourceScopeClass::BrowserSession,
        crate::actions::ActionIntegrationClass::Filesystem => ResourceScopeClass::FilePath,
        crate::actions::ActionIntegrationClass::App => ResourceScopeClass::App,
        crate::actions::ActionIntegrationClass::Workspace
        | crate::actions::ActionIntegrationClass::Messaging => {
            ResourceScopeClass::IntegrationAccount
        }
        crate::actions::ActionIntegrationClass::Code => ResourceScopeClass::Repository,
        crate::actions::ActionIntegrationClass::Analytics => ResourceScopeClass::DatabaseTable,
        crate::actions::ActionIntegrationClass::Search
        | crate::actions::ActionIntegrationClass::Network
        | crate::actions::ActionIntegrationClass::Commerce
        | crate::actions::ActionIntegrationClass::Media => ResourceScopeClass::ExternalResource,
        crate::actions::ActionIntegrationClass::Internal
        | crate::actions::ActionIntegrationClass::Unknown => ResourceScopeClass::Unknown,
    }
}

fn collect_scopes_from_value(
    key: Option<&str>,
    value: &serde_json::Value,
    read_scopes: &mut BTreeSet<ResourceScope>,
    write_scopes: &mut BTreeSet<ResourceScope>,
    exclusive_scopes: &mut BTreeSet<ResourceScope>,
    mutates: bool,
    depth: usize,
) {
    if depth > 8 {
        return;
    }

    match value {
        serde_json::Value::Object(map) => {
            for (child_key, child_value) in map {
                collect_scopes_from_value(
                    Some(child_key),
                    child_value,
                    read_scopes,
                    write_scopes,
                    exclusive_scopes,
                    mutates,
                    depth + 1,
                );
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter().take(32) {
                collect_scopes_from_value(
                    key,
                    item,
                    read_scopes,
                    write_scopes,
                    exclusive_scopes,
                    mutates,
                    depth + 1,
                );
            }
        }
        serde_json::Value::String(text) => {
            if let Some(scope) = scope_from_key_value(key, text) {
                insert_argument_scope(scope, read_scopes, write_scopes, exclusive_scopes, mutates);
            }
        }
        serde_json::Value::Number(number) => {
            if let Some(scope) = scope_from_key_value(key, &number.to_string()) {
                insert_argument_scope(scope, read_scopes, write_scopes, exclusive_scopes, mutates);
            }
        }
        serde_json::Value::Bool(value) => {
            if let Some(scope) = scope_from_key_value(key, &value.to_string()) {
                insert_argument_scope(scope, read_scopes, write_scopes, exclusive_scopes, mutates);
            }
        }
        serde_json::Value::Null => {}
    }
}

fn insert_argument_scope(
    scope: ResourceScope,
    read_scopes: &mut BTreeSet<ResourceScope>,
    write_scopes: &mut BTreeSet<ResourceScope>,
    exclusive_scopes: &mut BTreeSet<ResourceScope>,
    mutates: bool,
) {
    if !mutates {
        read_scopes.insert(scope);
        return;
    }
    match scope.class {
        ResourceScopeClass::BrowserSession
        | ResourceScopeClass::FilePath
        | ResourceScopeClass::App
        | ResourceScopeClass::Watcher
        | ResourceScopeClass::Repository
        | ResourceScopeClass::DatabaseTable => {
            read_scopes.insert(scope.clone());
            write_scopes.insert(scope.clone());
            exclusive_scopes.insert(scope);
        }
        _ => {
            read_scopes.insert(scope);
        }
    }
}

fn scope_from_key_value(key: Option<&str>, value: &str) -> Option<ResourceScope> {
    let key = key?;
    let tokens = structural_key_tokens(key);
    if tokens.is_empty() || value.trim().is_empty() {
        return None;
    }
    let class = if tokens.iter().any(|token| token == "browser" || token == "session") {
        ResourceScopeClass::BrowserSession
    } else if tokens.iter().any(|token| token == "watcher" || token == "monitor") {
        ResourceScopeClass::Watcher
    } else if tokens.iter().any(|token| token == "app" || token == "artifact") {
        ResourceScopeClass::App
    } else if tokens.iter().any(|token| token == "repo" || token == "repository") {
        ResourceScopeClass::Repository
    } else if tokens
        .iter()
        .any(|token| token == "database" || token == "table" || token == "schema")
    {
        ResourceScopeClass::DatabaseTable
    } else if tokens
        .iter()
        .any(|token| token == "path" || token == "file" || token == "document")
    {
        ResourceScopeClass::FilePath
    } else if tokens
        .iter()
        .any(|token| token == "account" || token == "integration" || token == "channel")
    {
        ResourceScopeClass::IntegrationAccount
    } else if tokens
        .iter()
        .any(|token| token == "url" || token == "uri" || token == "resource")
    {
        ResourceScopeClass::ExternalResource
    } else if tokens.iter().any(|token| token == "conversation" || token == "thread") {
        ResourceScopeClass::Conversation
    } else if tokens.iter().any(|token| token == "id") {
        ResourceScopeClass::Unknown
    } else {
        return None;
    };
    Some(ResourceScope::new(class, value))
}

fn structural_key_tokens(key: &str) -> Vec<String> {
    key.split(|ch: char| !ch.is_ascii_alphanumeric())
        .flat_map(|part| split_camel_case(part))
        .map(|part| part.to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

fn split_camel_case(part: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in part.chars() {
        if ch.is_ascii_uppercase() && !current.is_empty() {
            out.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn normalize_scope_atom(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '.' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_action(name: &str, capabilities: &[&str]) -> crate::actions::ActionDef {
        crate::actions::ActionDef {
            name: name.to_string(),
            capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            ..crate::actions::ActionDef::default()
        }
    }

    #[test]
    fn read_only_parallel_requires_distinct_resource_scopes() {
        let read_a = test_action("read_a", &["google_workspace"]);
        let read_b = test_action("read_b", &["search"]);
        let write = test_action("write_a", &["file_write"]);
        let actions = HashMap::from([
            (read_a.name.clone(), read_a),
            (read_b.name.clone(), read_b),
            (write.name.clone(), write),
        ]);

        let safe = vec![
            crate::core::llm::ToolCall {
                id: "a".to_string(),
                name: "read_a".to_string(),
                arguments: serde_json::json!({"file_id": "one"}),
            },
            crate::core::llm::ToolCall {
                id: "b".to_string(),
                name: "read_b".to_string(),
                arguments: serde_json::json!({"url": "https://example.com/two"}),
            },
        ];
        let overlapping = vec![
            crate::core::llm::ToolCall {
                id: "a".to_string(),
                name: "read_a".to_string(),
                arguments: serde_json::json!({"file_id": "same"}),
            },
            crate::core::llm::ToolCall {
                id: "b".to_string(),
                name: "read_b".to_string(),
                arguments: serde_json::json!({"file_id": "same"}),
            },
        ];
        let mut unsafe_write = safe.clone();
        unsafe_write.push(crate::core::llm::ToolCall {
            id: "c".to_string(),
            name: "write_a".to_string(),
            arguments: serde_json::json!({"path": "x"}),
        });

        assert!(tool_calls_are_parallel_safe(&safe, &actions));
        assert!(!tool_calls_are_parallel_safe(&overlapping, &actions));
        assert!(!tool_calls_are_parallel_safe(&unsafe_write, &actions));
    }
}
