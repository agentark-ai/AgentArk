//! Encrypted storage wrapper
//!
//! Provides transparent encryption for sensitive data in the database.
//! Content fields (fact text, message content, KV values)
//! are encrypted with AES-256-GCM before storage and decrypted on retrieval.
//! Non-content fields (timestamps, IDs, metadata) remain in plaintext for querying.

use super::entities::{approval_log, execution_trace, message};
use super::Storage;
use crate::crypto::KeyManager;
use anyhow::{anyhow, Result};
use parking_lot::RwLock;
#[cfg(test)]
use sea_orm::entity::prelude::PgVector;
use std::sync::Arc;

/// Encrypted storage that wraps the base storage
/// and encrypts sensitive fields before storing
#[derive(Clone)]
pub struct EncryptedStorage {
    storage: Storage,
    key_manager: Arc<RwLock<Arc<KeyManager>>>,
}

impl EncryptedStorage {
    /// Create a new encrypted storage
    pub fn new(storage: Storage, key_manager: Arc<KeyManager>) -> Self {
        crate::storage::install_storage_key_manager(key_manager.clone());
        Self {
            storage,
            key_manager: Arc::new(RwLock::new(key_manager)),
        }
    }

    pub fn current_key_manager(&self) -> Arc<KeyManager> {
        self.key_manager.read().clone()
    }

    pub fn replace_key_manager(&self, key_manager: Arc<KeyManager>) {
        crate::storage::install_storage_key_manager(key_manager.clone());
        *self.key_manager.write() = key_manager;
    }

    pub async fn reencrypt_all_sensitive_data(
        &self,
        old_key: Arc<KeyManager>,
        new_key: Arc<KeyManager>,
    ) -> Result<()> {
        const ROTATED_KV_KEYS: &[&str] = &[
            "user_profile",
            crate::core::observability::OBSERVABILITY_LOG_KEY,
            crate::sentinel::PULSE_LOG_KEY,
            crate::core::config::SETTINGS_CONFIG_KEY,
            crate::core::config::SETTINGS_SECRETS_KEY,
            crate::core::config::SETTINGS_SEARCH_KEY,
            crate::core::config::SETTINGS_RUNTIME_KEY,
            crate::core::config::SETTINGS_DISABLED_ACTIONS_KEY,
            crate::core::config::SETTINGS_ACTION_REVIEWS_KEY,
            crate::core::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
            crate::core::config::SETTINGS_APPROVED_PERMISSIONS_KEY,
        ];

        let lineage_record = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "fingerprint": new_key.fingerprint(),
            "recorded_at": chrono::Utc::now().to_rfc3339(),
        }))?;
        self.storage
            .reencrypt_sensitive_payloads(
                old_key.as_ref(),
                new_key.as_ref(),
                ROTATED_KV_KEYS,
                Some((
                    crate::core::config::SETTINGS_KEY_LINEAGE_KEY.to_string(),
                    lineage_record,
                )),
            )
            .await?;
        self.replace_key_manager(new_key);
        Ok(())
    }

    // ==================== Decrypt Helpers ====================

    /// Decrypt learned fact text when the caller hands us encrypted compatibility rows.
    fn decrypt_fact_content(
        &self,
        mut facts: Vec<super::LearnedFactRecord>,
    ) -> Vec<super::LearnedFactRecord> {
        let key_manager = self.current_key_manager();
        for fact in &mut facts {
            if let Ok(decrypted) = key_manager.decrypt_string(&fact.fact) {
                fact.fact = decrypted;
            }
            let value = super::learned_fact_value_from_content(fact.key.as_deref(), &fact.fact);
            if let Some(raw_key) = fact.key.clone() {
                let allow_value_suffix_repair = fact.memory_category
                    == crate::core::memory_schema::MEMORY_CATEGORY_PROFILE_FACT;
                if let Some((key, repaired_value)) =
                    crate::core::memory_schema::repair_memory_slot_key_and_value(
                        &raw_key,
                        &value,
                        allow_value_suffix_repair,
                    )
                {
                    fact.key = Some(key);
                    fact.value = repaired_value.unwrap_or(value);
                    continue;
                }
            }
            fact.value = value;
        }
        facts
    }

    // ==================== Learned Facts ====================

    /// Insert a learned fact with encrypted content in tests.
    #[cfg(test)]
    pub async fn insert_fact_encrypted(
        &self,
        id: &str,
        fact: &str,
        confidence: f32,
        sources: &str,
        embedding: Option<PgVector>,
        project_id: Option<&str>,
    ) -> Result<()> {
        let encrypted_fact = self.current_key_manager().encrypt_string(fact)?;
        self.storage
            .insert_fact(
                id,
                &encrypted_fact,
                confidence,
                sources,
                embedding,
                project_id,
            )
            .await
    }

    /// Get learned facts and decrypt their content when needed.
    pub async fn get_facts_decrypted(&self) -> Result<Vec<super::LearnedFactRecord>> {
        let facts = self.storage.get_facts().await?;
        Ok(self.decrypt_fact_content(facts))
    }

    /// Get learned facts by project and decrypt their content when needed.
    pub async fn get_facts_by_project_decrypted(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<super::LearnedFactRecord>> {
        let facts = self
            .storage
            .get_facts_by_project(limit, offset, project_id)
            .await?;
        Ok(self.decrypt_fact_content(facts))
    }

    /// Get learned memory rows by semantic category and decrypt their content when needed.
    pub async fn get_facts_by_project_and_category_decrypted(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        category: &str,
    ) -> Result<Vec<super::LearnedFactRecord>> {
        let facts = self
            .storage
            .get_facts_by_project_and_category(limit, offset, project_id, category)
            .await?;
        Ok(self.decrypt_fact_content(facts))
    }

    // ==================== Encrypted KV Store ====================

    /// Set an encrypted value in the KV store
    pub async fn set_encrypted(&self, key: &str, value: &[u8]) -> Result<()> {
        let encrypted = self.current_key_manager().encrypt(value)?;
        self.storage.set(key, &encrypted).await
    }

    /// Get and decrypt a value from the KV store
    pub async fn get_decrypted(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let key_manager = self.current_key_manager();
        match self.storage.get(key).await? {
            Some(encrypted) => match key_manager.decrypt(&encrypted) {
                Ok(decrypted) => Ok(Some(decrypted)),
                Err(error) => Err(anyhow!(
                    "Failed to decrypt encrypted KV value '{}': {}",
                    key,
                    error
                )),
            },
            None => Ok(None),
        }
    }

    // ==================== Encrypted Messages ====================

    pub async fn insert_message_encrypted(&self, msg: &message::Model) -> Result<()> {
        self.storage.insert_message(msg).await
    }

    pub async fn insert_message_encrypted_if_absent(&self, msg: &message::Model) -> Result<bool> {
        self.storage.insert_message_if_absent(msg).await
    }

    pub async fn get_messages_decrypted(
        &self,
        conversation_id: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<message::Model>> {
        self.storage
            .get_messages(conversation_id, limit, offset)
            .await
    }

    pub async fn get_recent_messages_decrypted(
        &self,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        self.storage
            .get_recent_messages(conversation_id, limit)
            .await
    }

    pub async fn get_recent_user_messages_decrypted(
        &self,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        self.storage.get_recent_user_messages(limit).await
    }

    // ==================== Encrypted Approval Log ====================

    pub async fn upsert_approval_request_encrypted(
        &self,
        id: &str,
        action_name: &str,
        arguments: &str,
        rule_name: &str,
        requested_at: &str,
    ) -> Result<()> {
        self.storage
            .upsert_approval_request(id, action_name, arguments, rule_name, requested_at)
            .await
    }

    pub async fn get_approval_log_decrypted(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<approval_log::Model>> {
        self.storage.get_approval_log(limit, offset).await
    }

    // ==================== Encrypted Execution Traces ====================

    pub async fn insert_execution_trace_encrypted(
        &self,
        trace: &crate::core::ExecutionTrace,
    ) -> Result<()> {
        self.storage.insert_execution_trace(trace).await
    }

    pub async fn get_execution_trace_decrypted(
        &self,
        id: &str,
    ) -> Result<Option<execution_trace::Model>> {
        self.storage.get_execution_trace(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn reencrypt_all_sensitive_data_updates_rows_and_live_key() {
        let _temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        let old_key = Arc::new(
            KeyManager::from_password("old-password", &[1_u8; crate::crypto::SALT_LEN]).unwrap(),
        );
        let new_key = Arc::new(
            KeyManager::from_password("new-password", &[2_u8; crate::crypto::SALT_LEN]).unwrap(),
        );
        let encrypted_storage = EncryptedStorage::new(storage.clone(), old_key.clone());
        let clone = encrypted_storage.clone();

        encrypted_storage
            .insert_fact_encrypted("fact-1", "fact secret", 0.9, "[]", None, None)
            .await
            .unwrap();
        encrypted_storage
            .set_encrypted("user_profile", br#"{"name":"Ada"}"#)
            .await
            .unwrap();
        encrypted_storage
            .set_encrypted(
                crate::core::observability::OBSERVABILITY_LOG_KEY,
                br#"[{"id":"obs-1","timestamp":"2026-03-19T00:00:00Z","level":"info","event":"test","message":"ok","provider":"langtrace","endpoint":"https://example.com","trace_id":null,"status_code":200}]"#,
            )
            .await
            .unwrap();
        encrypted_storage
            .set_encrypted(
                crate::sentinel::PULSE_LOG_KEY,
                br#"[{"timestamp":"2026-03-19T00:00:00Z","status":"ok","message":"healthy","summary":"","flags":[],"overdue_tasks":0,"failed_tasks":0,"details":{"pending_tasks":0,"running_tasks":0,"completed_tasks":0,"total_tasks":0,"active_watchers":0,"total_memories":0,"overdue_list":[],"failed_list":[],"uptime_secs":0,"health_checks":[],"security":null,"deployed_apps":[],"doctor_findings":[],"doctor_score":100}}]"#,
            )
            .await
            .unwrap();

        let raw_fact_before = storage
            .get_facts()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == "fact-1")
            .unwrap()
            .fact;
        assert!(old_key.decrypt_string(&raw_fact_before).is_ok());
        assert!(new_key.decrypt_string(&raw_fact_before).is_err());

        encrypted_storage
            .reencrypt_all_sensitive_data(old_key.clone(), new_key.clone())
            .await
            .unwrap();

        let raw_fact_after = storage
            .get_facts()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == "fact-1")
            .unwrap()
            .fact;
        assert!(old_key.decrypt_string(&raw_fact_after).is_err());
        assert_eq!(
            new_key.decrypt_string(&raw_fact_after).unwrap(),
            "fact secret"
        );

        let raw_profile = storage.get("user_profile").await.unwrap().unwrap();
        assert!(old_key.decrypt(&raw_profile).is_err());
        assert_eq!(
            new_key.decrypt(&raw_profile).unwrap(),
            br#"{"name":"Ada"}"#.to_vec()
        );
        let lineage_raw = storage
            .get(crate::core::config::SETTINGS_KEY_LINEAGE_KEY)
            .await
            .unwrap()
            .expect("lineage metadata should be written during re-encryption");
        let lineage: serde_json::Value = serde_json::from_slice(&lineage_raw).unwrap();
        assert_eq!(
            lineage
                .get("fingerprint")
                .and_then(|value| value.as_str())
                .unwrap(),
            new_key.fingerprint()
        );
        let raw_observability = storage
            .get(crate::core::observability::OBSERVABILITY_LOG_KEY)
            .await
            .unwrap()
            .unwrap();
        assert!(old_key.decrypt(&raw_observability).is_err());
        assert!(new_key.decrypt(&raw_observability).is_ok());
        let raw_pulse = storage
            .get(crate::sentinel::PULSE_LOG_KEY)
            .await
            .unwrap()
            .unwrap();
        assert!(old_key.decrypt(&raw_pulse).is_err());
        assert!(new_key.decrypt(&raw_pulse).is_ok());

        let facts = clone.get_facts_decrypted().await.unwrap();
        assert_eq!(facts[0].fact, "fact secret");
        assert_eq!(
            clone.get_decrypted("user_profile").await.unwrap().unwrap(),
            br#"{"name":"Ada"}"#.to_vec()
        );
    }
}
