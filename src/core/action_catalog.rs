use std::collections::HashSet;

use sea_orm::entity::prelude::PgVector;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::actions::{ActionDef, ActionSource};

#[derive(Debug, Clone)]
pub(crate) struct ActionCatalogDescriptor {
    pub action_name: String,
    pub source: String,
    pub version: String,
    pub descriptor_hash: String,
    pub descriptor_text: String,
    pub metadata_json: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActionCatalogSyncStats {
    pub actions_seen: usize,
    pub embedded: usize,
    pub reused_embeddings: usize,
    pub missing_embeddings: usize,
    pub stale_disabled: u64,
    pub embedding_failures: usize,
}

fn action_source_key(source: &ActionSource) -> &'static str {
    match source {
        ActionSource::System => "system",
        ActionSource::Bundled => "bundled",
        ActionSource::Custom => "custom",
    }
}

fn schema_type_label(value: &serde_json::Value) -> String {
    if let Some(typ) = value.get("type").and_then(|inner| inner.as_str()) {
        return typ.to_string();
    }
    if let Some(types) = value.get("type").and_then(|inner| inner.as_array()) {
        let joined = types
            .iter()
            .filter_map(|inner| inner.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if !joined.is_empty() {
            return joined;
        }
    }
    if value.get("properties").is_some() {
        "object".to_string()
    } else if value.get("items").is_some() {
        "array".to_string()
    } else {
        "any".to_string()
    }
}

fn direct_required_fields(schema: &serde_json::Value) -> HashSet<String> {
    schema
        .get("required")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(|value| value.to_string())
        .collect()
}

fn composed_required_fields(schema: &serde_json::Value) -> HashSet<String> {
    let mut out = HashSet::new();
    for keyword in ["oneOf", "anyOf", "allOf"] {
        let Some(items) = schema.get(keyword).and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            out.extend(direct_required_fields(item));
            out.extend(composed_required_fields(item));
        }
    }
    out
}

fn direct_required_field_list(schema: &serde_json::Value) -> Vec<String> {
    let mut fields = schema
        .get("required")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    fields.sort();
    fields.dedup();
    fields
}

fn collect_required_shapes(
    schema: &serde_json::Value,
    label: &str,
    out: &mut Vec<String>,
    max_shapes: usize,
) {
    if out.len() >= max_shapes {
        return;
    }

    let direct = direct_required_field_list(schema);
    if !direct.is_empty() {
        out.push(format!("{label}: {}", direct.join("+")));
        if out.len() >= max_shapes {
            return;
        }
    }

    for keyword in ["oneOf", "anyOf", "allOf"] {
        let Some(items) = schema.get(keyword).and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            if out.len() >= max_shapes {
                return;
            }
            collect_required_shapes(item, keyword, out, max_shapes);
        }
    }
}

pub(crate) fn action_schema_required_shape_descriptions(
    schema: &serde_json::Value,
    max_shapes: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    collect_required_shapes(schema, "required", &mut out, max_shapes);
    out.sort();
    out.dedup();
    out.truncate(max_shapes);
    out
}

fn collect_schema_field_descriptions(
    schema: &serde_json::Value,
    prefix: &str,
    out: &mut Vec<String>,
    max_fields: usize,
) {
    if out.len() >= max_fields {
        return;
    }

    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        if let Some(items) = schema.get("items") {
            collect_schema_field_descriptions(items, prefix, out, max_fields);
        }
        return;
    };

    let required = direct_required_fields(schema);
    let conditional_required = composed_required_fields(schema);
    let mut keys = properties.keys().collect::<Vec<_>>();
    keys.sort_by(|left, right| {
        let left_required = required.contains(left.as_str());
        let right_required = required.contains(right.as_str());
        let left_conditional = conditional_required.contains(left.as_str());
        let right_conditional = conditional_required.contains(right.as_str());
        right_required
            .cmp(&left_required)
            .then_with(|| right_conditional.cmp(&left_conditional))
            .then_with(|| left.cmp(right))
    });

    for key in keys {
        if out.len() >= max_fields {
            return;
        }
        let Some(property) = properties.get(key) else {
            continue;
        };
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        let typ = schema_type_label(property);
        let required_label = if required.contains(key.as_str()) {
            " required"
        } else if conditional_required.contains(key.as_str()) {
            " conditionally_required"
        } else {
            ""
        };
        let description = property
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        if description.is_empty() {
            out.push(format!("{path}: {typ}{required_label}"));
        } else {
            out.push(format!("{path}: {typ}{required_label} - {description}"));
        }

        collect_schema_field_descriptions(property, &path, out, max_fields);
    }
}

pub(crate) fn action_schema_field_descriptions(
    schema: &serde_json::Value,
    max_fields: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    collect_schema_field_descriptions(schema, "", &mut out, max_fields);
    out
}

fn descriptor_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn build_action_catalog_descriptor(action: &ActionDef) -> ActionCatalogDescriptor {
    let planner_metadata = action.planner_metadata();
    let schema_fields = action_schema_field_descriptions(&action.input_schema, 32);
    let source = action_source_key(&action.source).to_string();
    let metadata_json = json!({
        "source": source.clone(),
        "version": action.version.clone(),
        "capabilities": action.capabilities.clone(),
        "planner_metadata": planner_metadata,
        "authorization": {
            "requires_auth": action.authorization.requires_auth,
            "risk_level": action.authorization.risk_level.clone(),
            "human_approval_required": action.authorization.human_approval.required,
            "outbound": action.authorization.outbound.clone(),
        },
        "schema_fields": schema_fields.clone(),
    });

    let mut descriptor = String::new();
    descriptor.push_str(&format!("action: {}\n", action.name.trim()));
    descriptor.push_str(&format!("source: {}\n", source));
    descriptor.push_str(&format!("version: {}\n", action.version.trim()));
    descriptor.push_str(&format!("description: {}\n", action.description.trim()));
    if !action.capabilities.is_empty() {
        descriptor.push_str(&format!(
            "capabilities: {}\n",
            action.capabilities.join(", ")
        ));
    }
    descriptor.push_str(&format!(
        "planner_metadata: {}\n",
        serde_json::to_string(&planner_metadata).unwrap_or_default()
    ));
    if !schema_fields.is_empty() {
        descriptor.push_str("schema_fields:\n");
        for field in &schema_fields {
            descriptor.push_str("- ");
            descriptor.push_str(field);
            descriptor.push('\n');
        }
    }
    let required_shapes = action_schema_required_shape_descriptions(&action.input_schema, 12);
    if !required_shapes.is_empty() {
        descriptor.push_str("required_shapes:\n");
        for shape in &required_shapes {
            descriptor.push_str("- ");
            descriptor.push_str(shape);
            descriptor.push('\n');
        }
    }

    ActionCatalogDescriptor {
        action_name: action.name.clone(),
        source,
        version: action.version.clone(),
        descriptor_hash: descriptor_hash(&descriptor),
        descriptor_text: descriptor,
        metadata_json,
    }
}

pub(crate) fn action_catalog_embedding_has_default_dim(embedding: &PgVector) -> bool {
    embedding.as_slice().len() == crate::actions::ACTION_CATALOG_EMBEDDING_DIM
}

pub(crate) fn action_catalog_entry_needs_embedding(
    descriptor: &ActionCatalogDescriptor,
    existing: Option<&crate::storage::action_catalog_index::Model>,
) -> bool {
    existing
        .map(|row| {
            row.descriptor_hash != descriptor.descriptor_hash
                || row
                    .embedding
                    .as_ref()
                    .map(|embedding| !action_catalog_embedding_has_default_dim(embedding))
                    .unwrap_or(true)
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Topic or objective to inspect"
                    },
                    "options": {
                        "type": "object",
                        "properties": {
                            "depth": {
                                "type": "string",
                                "description": "How much evidence to gather"
                            }
                        }
                    }
                }
            }),
            capabilities: vec!["research".to_string()],
            sandbox_mode: None,
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }
    }

    #[test]
    fn descriptor_includes_schema_field_descriptions() {
        let descriptor = build_action_catalog_descriptor(&action("research", "Gather evidence"));

        assert!(descriptor
            .descriptor_text
            .contains("query: string required"));
        assert!(descriptor.descriptor_text.contains("Topic or objective"));
        assert!(descriptor.descriptor_text.contains("options.depth"));
    }

    #[test]
    fn schema_summary_prioritizes_composed_required_shapes() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string"},
                "allow_duplicate": {"type": "boolean"},
                "at": {"type": "string"},
                "cron": {"type": "string"},
                "report_to": {"type": "string"},
                "task": {"type": "string"},
                "task_id": {"type": "string"}
            },
            "oneOf": [
                {"required": ["task", "cron"]},
                {"required": ["task", "at"]},
                {"required": ["task_id", "cron"]},
                {"required": ["task_id", "at"]}
            ]
        });

        let fields = action_schema_field_descriptions(&schema, 4);
        let joined = fields.join("\n");
        assert!(joined.contains("task: string conditionally_required"));
        assert!(joined.contains("cron: string conditionally_required"));
        assert!(joined.contains("at: string conditionally_required"));
        assert!(joined.contains("task_id: string conditionally_required"));
    }

    #[test]
    fn descriptor_hash_changes_when_descriptor_changes() {
        let first = build_action_catalog_descriptor(&action("research", "Gather evidence"));
        let second = build_action_catalog_descriptor(&action("research", "Gather cited evidence"));

        assert_ne!(first.descriptor_hash, second.descriptor_hash);
    }

    #[test]
    fn entry_needs_embedding_when_descriptor_changed_missing_or_wrong_dimension() {
        let descriptor = build_action_catalog_descriptor(&action("research", "Gather evidence"));
        let matching = crate::storage::action_catalog_index::Model {
            action_name: descriptor.action_name.clone(),
            source: descriptor.source.clone(),
            version: descriptor.version.clone(),
            descriptor_hash: descriptor.descriptor_hash.clone(),
            descriptor_text: descriptor.descriptor_text.clone(),
            enabled: true,
            metadata_json: descriptor.metadata_json.clone(),
            embedding: Some(PgVector::from(vec![
                0.0;
                crate::actions::ACTION_CATALOG_EMBEDDING_DIM
            ])),
            updated_at: "now".to_string(),
        };
        assert!(!action_catalog_entry_needs_embedding(
            &descriptor,
            Some(&matching)
        ));

        let mut changed = matching.clone();
        changed.descriptor_hash = "changed".to_string();
        assert!(action_catalog_entry_needs_embedding(
            &descriptor,
            Some(&changed)
        ));

        let mut missing = matching.clone();
        missing.embedding = None;
        assert!(action_catalog_entry_needs_embedding(
            &descriptor,
            Some(&missing)
        ));

        let mut wrong_dim = matching;
        wrong_dim.embedding = Some(PgVector::from(vec![0.0; 16]));
        assert!(action_catalog_entry_needs_embedding(
            &descriptor,
            Some(&wrong_dim)
        ));
    }
}
