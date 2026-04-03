//! Encrypted storage wrapper
//!
//! Provides transparent encryption for sensitive data in the database.
//! Content fields (episode content, fact text, message content, KV values)
//! are encrypted with AES-256-GCM before storage and decrypted on retrieval.
//! Non-content fields (timestamps, IDs, metadata) remain in plaintext for querying.

use super::entities::{approval_log, episode, execution_trace, message, semantic_fact};
use super::Storage;
use crate::crypto::KeyManager;
use anyhow::Result;
use parking_lot::RwLock;
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
        ];

        self.storage
            .reencrypt_sensitive_payloads(old_key.as_ref(), new_key.as_ref(), ROTATED_KV_KEYS)
            .await?;
        self.replace_key_manager(new_key);
        Ok(())
    }

    // ==================== Decrypt Helpers ====================

    /// Decrypt the content field of episodes, falling back to plaintext for legacy data
    fn decrypt_episode_content(&self, mut episodes: Vec<episode::Model>) -> Vec<episode::Model> {
        let key_manager = self.current_key_manager();
        for ep in &mut episodes {
            if let Ok(decrypted) = key_manager.decrypt_string(&ep.content) {
                ep.content = decrypted;
            }
            // If decrypt fails, content is already plaintext (legacy) — leave as-is
        }
        episodes
    }

    /// Decrypt the fact field of semantic facts, falling back to plaintext for legacy data
    fn decrypt_fact_content(
        &self,
        mut facts: Vec<semantic_fact::Model>,
    ) -> Vec<semantic_fact::Model> {
        let key_manager = self.current_key_manager();
        for f in &mut facts {
            if let Ok(decrypted) = key_manager.decrypt_string(&f.fact) {
                f.fact = decrypted;
            }
        }
        facts
    }

    // ==================== Encrypted Episodes ====================

    /// Insert an episode with encrypted content
    pub async fn insert_episode_encrypted(
        &self,
        id: &str,
        content: &str,
        context: &str,
        embedding: Option<PgVector>,
        importance: f32,
        project_id: Option<&str>,
    ) -> Result<()> {
        let encrypted_content = self.current_key_manager().encrypt_string(content)?;
        self.storage
            .insert_episode(
                id,
                &encrypted_content,
                context,
                embedding,
                importance,
                project_id,
            )
            .await
    }

    /// Get all episodes for scoring and decrypt content
    pub async fn get_all_episodes_for_scoring_decrypted(&self) -> Result<Vec<episode::Model>> {
        let episodes = self.storage.get_all_episodes_for_scoring().await?;
        Ok(self.decrypt_episode_content(episodes))
    }

    /// Get all episodes for scoring by project and decrypt content
    pub async fn get_all_episodes_for_scoring_by_project_decrypted(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<episode::Model>> {
        let episodes = self
            .storage
            .get_all_episodes_for_scoring_by_project(project_id)
            .await?;
        Ok(self.decrypt_episode_content(episodes))
    }

    pub async fn get_episodes_by_ids_decrypted(&self, ids: &[String]) -> Result<Vec<episode::Model>> {
        let episodes = self.storage.get_episodes_by_ids(ids).await?;
        Ok(self.decrypt_episode_content(episodes))
    }

    /// Get unconsolidated episodes and decrypt content
    pub async fn get_unconsolidated_episodes_decrypted(
        &self,
        limit: u64,
    ) -> Result<Vec<episode::Model>> {
        let episodes = self.storage.get_unconsolidated_episodes(limit).await?;
        Ok(self.decrypt_episode_content(episodes))
    }

    /// Get episodes by project and decrypt content
    pub async fn get_episodes_by_project_decrypted(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<episode::Model>> {
        let episodes = self
            .storage
            .get_episodes_by_project(limit, offset, project_id)
            .await?;
        Ok(self.decrypt_episode_content(episodes))
    }

    // ==================== Encrypted Semantic Facts ====================

    /// Insert a semantic fact with encrypted content
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

    /// Get facts and decrypt their content
    pub async fn get_facts_decrypted(&self) -> Result<Vec<semantic_fact::Model>> {
        let facts = self.storage.get_facts().await?;
        Ok(self.decrypt_fact_content(facts))
    }

    /// Get facts by project and decrypt their content (paginated)
    pub async fn get_facts_by_project_decrypted(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<semantic_fact::Model>> {
        let facts = self
            .storage
            .get_facts_by_project(limit, offset, project_id)
            .await?;
        Ok(self.decrypt_fact_content(facts))
    }

    /// Get only global-scope facts and decrypt their content.
    pub async fn get_global_facts_decrypted(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<semantic_fact::Model>> {
        let facts = self.storage.get_global_facts(limit, offset).await?;
        Ok(self.decrypt_fact_content(facts))
    }

    pub async fn get_facts_by_ids_decrypted(&self, ids: &[String]) -> Result<Vec<semantic_fact::Model>> {
        let facts = self.storage.get_facts_by_ids(ids).await?;
        Ok(self.decrypt_fact_content(facts))
    }

    pub async fn count_facts(&self, project_id: Option<&str>) -> Result<u64> {
        self.storage.count_facts(project_id).await
    }

    pub async fn count_global_facts(&self) -> Result<u64> {
        self.storage.count_global_facts().await
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
            Some(encrypted) => {
                match key_manager.decrypt(&encrypted) {
                    Ok(decrypted) => Ok(Some(decrypted)),
                    Err(_) => Ok(Some(encrypted)), // Legacy unencrypted data
                }
            }
            None => Ok(None),
        }
    }

    // ==================== Encrypted Messages ====================

    pub async fn insert_message_encrypted(&self, msg: &message::Model) -> Result<()> {
        self.storage.insert_message(msg).await
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
            .insert_episode_encrypted("ep-1", "episode secret", "ctx", None, 0.5, None)
            .await
            .unwrap();
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

        let raw_episode_before = storage
            .get_all_episodes_for_scoring()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == "ep-1")
            .unwrap()
            .content;
        assert!(old_key.decrypt_string(&raw_episode_before).is_ok());
        assert!(new_key.decrypt_string(&raw_episode_before).is_err());

        encrypted_storage
            .reencrypt_all_sensitive_data(old_key.clone(), new_key.clone())
            .await
            .unwrap();

        let raw_episode_after = storage
            .get_all_episodes_for_scoring()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == "ep-1")
            .unwrap()
            .content;
        assert!(old_key.decrypt_string(&raw_episode_after).is_err());
        assert_eq!(
            new_key.decrypt_string(&raw_episode_after).unwrap(),
            "episode secret"
        );

        let raw_profile = storage.get("user_profile").await.unwrap().unwrap();
        assert!(old_key.decrypt(&raw_profile).is_err());
        assert_eq!(
            new_key.decrypt(&raw_profile).unwrap(),
            br#"{"name":"Ada"}"#.to_vec()
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

        let episodes = clone
            .get_all_episodes_for_scoring_decrypted()
            .await
            .unwrap();
        assert_eq!(episodes[0].content, "episode secret");
        let facts = clone.get_facts_decrypted().await.unwrap();
        assert_eq!(facts[0].fact, "fact secret");
        assert_eq!(
            clone.get_decrypted("user_profile").await.unwrap().unwrap(),
            br#"{"name":"Ada"}"#.to_vec()
        );
    }
}
