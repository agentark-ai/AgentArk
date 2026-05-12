use super::*;

const CONTEXT_LEDGER_KEY_PREFIX: &str = "conversation_context_ledger_v1:";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct LedgerResourceRef {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ConversationContextLedger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_app: Option<LedgerResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_watcher: Option<LedgerResourceRef>,
    #[serde(default)]
    pub pending_approvals: Vec<LedgerResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_changed_artifact: Option<LedgerResourceRef>,
    #[serde(default)]
    pub recent_resources: Vec<LedgerResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl ConversationContextLedger {
    pub(super) fn compact_for_prompt(&self) -> serde_json::Value {
        serde_json::json!({
            "active_app": &self.active_app,
            "active_watcher": &self.active_watcher,
            "pending_approvals": self.pending_approvals.iter().take(6).collect::<Vec<_>>(),
            "current_objective": &self.current_objective,
            "last_changed_artifact": &self.last_changed_artifact,
            "recent_resources": self.recent_resources.iter().take(10).collect::<Vec<_>>(),
            "updated_at": &self.updated_at,
        })
    }

    fn merge_tool_facts(
        &mut self,
        facts: &super::tool_facts::ToolFacts,
        semantic_turn: Option<&super::semantic_turn::SemanticTurnBundle>,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(bundle) = semantic_turn {
            let summary = bundle.plan.turn_summary.trim();
            if !summary.is_empty() {
                self.current_objective = Some(safe_truncate(summary, 260));
            }
        }
        for resource in &facts.resources {
            let ledger_ref = LedgerResourceRef {
                kind: format!("{:?}", &resource.kind).to_ascii_lowercase(),
                id: resource.id.clone(),
                label: resource.label.clone(),
                status: facts.status.clone(),
                updated_at: Some(now.clone()),
            };
            match &resource.kind {
                super::tool_facts::ToolFactResourceKind::App => {
                    self.active_app = Some(ledger_ref.clone());
                    self.last_changed_artifact = Some(ledger_ref.clone());
                }
                super::tool_facts::ToolFactResourceKind::Watcher
                | super::tool_facts::ToolFactResourceKind::BackgroundSession => {
                    self.active_watcher = Some(ledger_ref.clone());
                }
                super::tool_facts::ToolFactResourceKind::Approval => {
                    upsert_ledger_ref(&mut self.pending_approvals, ledger_ref.clone(), 8);
                }
                _ => {}
            }
            upsert_ledger_ref(&mut self.recent_resources, ledger_ref, 16);
        }
        self.updated_at = Some(now);
    }
}

impl Agent {
    pub(super) async fn load_conversation_context_ledger(
        &self,
        conversation_id: &str,
    ) -> ConversationContextLedger {
        let key = context_ledger_key(conversation_id);
        match self.storage.get(&key).await {
            Ok(Some(raw)) => serde_json::from_slice::<ConversationContextLedger>(&raw)
                .unwrap_or_default(),
            Ok(None) => ConversationContextLedger::default(),
            Err(error) => {
                tracing::debug!("Failed to load conversation context ledger: {}", error);
                ConversationContextLedger::default()
            }
        }
    }

    pub(super) async fn record_tool_facts_in_context_ledger(
        &self,
        conversation_id: &str,
        facts: &super::tool_facts::ToolFacts,
        semantic_turn: Option<&super::semantic_turn::SemanticTurnBundle>,
    ) {
        if facts.is_empty() {
            return;
        }
        let mut ledger = self.load_conversation_context_ledger(conversation_id).await;
        ledger.merge_tool_facts(facts, semantic_turn);
        let key = context_ledger_key(conversation_id);
        match serde_json::to_vec(&ledger) {
            Ok(raw) => {
                if let Err(error) = self.storage.set(&key, &raw).await {
                    tracing::debug!("Failed to save conversation context ledger: {}", error);
                }
            }
            Err(error) => {
                tracing::debug!("Failed to serialize conversation context ledger: {}", error);
            }
        }
    }
}

fn upsert_ledger_ref(items: &mut Vec<LedgerResourceRef>, item: LedgerResourceRef, limit: usize) {
    items.retain(|existing| !(existing.kind == item.kind && existing.id == item.id));
    items.insert(0, item);
    items.truncate(limit);
}

fn context_ledger_key(conversation_id: &str) -> String {
    format!("{CONTEXT_LEDGER_KEY_PREFIX}{}", conversation_id.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_promotes_watcher_facts_to_active_watcher() {
        let mut ledger = ConversationContextLedger::default();
        let facts = super::super::tool_facts::extract_tool_facts(
            "watch",
            &serde_json::json!({
                "status": "completed",
                "watcher_id": "w1"
            }),
        );

        ledger.merge_tool_facts(&facts, None);

        assert_eq!(
            ledger.active_watcher.as_ref().map(|item| item.id.as_str()),
            Some("w1")
        );
        assert!(ledger.recent_resources.iter().any(|item| item.id == "w1"));
    }
}
