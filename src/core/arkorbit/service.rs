//! Filesystem-backed ArkOrbit service.
//!
//! Orbits are folders under `<DATA_DIR>/arkorbit/L2/orbits/<id>/`. No
//! ArkOrbit database tables are created or queried in this redesign.

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::storage::Storage;

use super::models::{
    Orbit, OrbitChatMessage, OrbitChatTranscriptSummary, OrbitFileEntry, OrbitManifest, OrbitUpdate,
};
use super::store::{validate_readable_orbit_path, LayeredStore, ResolvedModule};

#[derive(Clone)]
pub struct ArkOrbitService {
    store: Arc<LayeredStore>,
}

impl ArkOrbitService {
    pub fn with_filesystem(_storage: Storage, data_dir: &Path) -> Self {
        Self {
            store: Arc::new(LayeredStore::new(data_dir)),
        }
    }

    fn now() -> String {
        Utc::now().to_rfc3339()
    }

    async fn run_store_blocking<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<LayeredStore>) -> Result<T> + Send + 'static,
    {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || operation(store)).await?
    }

    async fn ensure_orbit_dir_async(&self, orbit_id: &str) -> Result<PathBuf> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let dir = self.store.orbit_dir(orbit_id);
        tokio::fs::create_dir_all(dir.join("mod")).await?;
        tokio::fs::create_dir_all(dir.join("data")).await?;
        tokio::fs::create_dir_all(dir.join("assets")).await?;
        tokio::fs::create_dir_all(dir.join(".tmp")).await?;
        Ok(dir)
    }

    fn require_non_empty(value: &str, field: &str) -> Result<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("ArkOrbit: '{}' must be a non-empty string", field);
        }
        Ok(trimmed.to_string())
    }

    fn normalize_optional(value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
    }

    fn normalized_orbit_name(value: &str) -> String {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    async fn ensure_unique_orbit_name(
        &self,
        user_id: &str,
        name: &str,
        excluding_orbit_id: Option<&str>,
    ) -> Result<()> {
        let target = Self::normalized_orbit_name(name);
        for orbit in self.list_orbits(user_id).await? {
            if orbit.user_id != user_id && !orbit.user_id.is_empty() {
                continue;
            }
            if excluding_orbit_id.is_some_and(|id| id == orbit.id) {
                continue;
            }
            if Self::normalized_orbit_name(&orbit.name) == target {
                bail!("ArkOrbit: orbit name '{}' already exists", name);
            }
        }
        Ok(())
    }

    pub async fn list_orbits(&self, user_id: &str) -> Result<Vec<Orbit>> {
        let user_id = Self::require_non_empty(user_id, "user_id")?;
        let mut orbits = self
            .run_store_blocking(|store| {
                let mut orbits = Vec::new();
                for id in store.list_orbit_dirs()? {
                    match store.read_orbit_manifest(&id) {
                        Ok(manifest) => orbits.push(Orbit::from(manifest)),
                        Err(error) => tracing::warn!(
                            target: "arkorbit.fs",
                            orbit_id = %id,
                            error = %error,
                            "Skipping unreadable ArkOrbit manifest"
                        ),
                    }
                }
                Ok(orbits)
            })
            .await?;
        if orbits.is_empty() {
            let orbit = self
                .create_orbit_internal(&user_id, "Home", None, None, None, true)
                .await?;
            return Ok(vec![orbit]);
        }
        orbits.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(orbits)
    }

    pub async fn get_orbit(&self, orbit_id: &str) -> Result<Option<Orbit>> {
        let orbit_id = Self::require_non_empty(orbit_id, "orbit_id")?;
        let orbit_id_for_read = orbit_id.clone();
        match self
            .run_store_blocking(move |store| store.read_orbit_manifest(&orbit_id_for_read))
            .await
        {
            Ok(manifest) => Ok(Some(Orbit::from(manifest))),
            Err(error) => {
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
                {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn create_orbit(
        &self,
        user_id: &str,
        name: &str,
        icon: Option<String>,
        color: Option<String>,
        agent_instructions: Option<String>,
    ) -> Result<Orbit> {
        let user_id = Self::require_non_empty(user_id, "user_id")?;
        let name = Self::require_non_empty(name, "name")?;
        self.ensure_unique_orbit_name(&user_id, &name, None).await?;
        self.create_orbit_internal(
            &user_id,
            &name,
            Self::normalize_optional(icon),
            Self::normalize_optional(color),
            Self::normalize_optional(agent_instructions),
            false,
        )
        .await
    }

    async fn create_orbit_internal(
        &self,
        user_id: &str,
        name: &str,
        icon: Option<String>,
        color: Option<String>,
        agent_instructions: Option<String>,
        is_default: bool,
    ) -> Result<Orbit> {
        let now = Self::now();
        let orbit = Orbit {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            name: name.to_string(),
            is_default,
            icon,
            color,
            agent_instructions,
            created_at: now.clone(),
            updated_at: now,
        };
        let manifest = OrbitManifest::from(&orbit);
        let orbit_id = orbit.id.clone();
        self.run_store_blocking(move |store| {
            store.write_orbit_manifest(&manifest)?;
            store.write_default_index(&orbit_id)?;
            Ok(())
        })
        .await?;
        Ok(orbit)
    }

    pub async fn update_orbit(&self, orbit_id: &str, patch: OrbitUpdate) -> Result<Orbit> {
        let orbit_id = Self::require_non_empty(orbit_id, "orbit_id")?;
        let mut orbit = self
            .get_orbit(&orbit_id)
            .await?
            .ok_or_else(|| anyhow!("ArkOrbit: orbit '{}' not found", orbit_id))?;
        if let Some(name) = patch.name {
            let name = Self::require_non_empty(&name, "name")?;
            self.ensure_unique_orbit_name(&orbit.user_id, &name, Some(&orbit.id))
                .await?;
            orbit.name = name;
        }
        if let Some(icon) = patch.icon {
            orbit.icon = Self::normalize_optional(icon);
        }
        if let Some(color) = patch.color {
            orbit.color = Self::normalize_optional(color);
        }
        if let Some(agent_instructions) = patch.agent_instructions {
            orbit.agent_instructions = Self::normalize_optional(agent_instructions);
        }
        orbit.updated_at = Self::now();
        let manifest = OrbitManifest::from(&orbit);
        self.run_store_blocking(move |store| store.write_orbit_manifest(&manifest))
            .await?;
        Ok(orbit)
    }

    pub async fn delete_orbit(&self, orbit_id: &str) -> Result<()> {
        let orbit_id = Self::require_non_empty(orbit_id, "orbit_id")?;
        self.run_store_blocking(move |store| store.remove_orbit(&orbit_id))
            .await
    }

    pub async fn read_orbit_index_async(&self, orbit_id: &str) -> Result<Vec<u8>> {
        let orbit_id = orbit_id.to_string();
        self.run_store_blocking(move |store| store.read_orbit_index(&orbit_id))
            .await
    }

    pub async fn resolve_module_async(
        &self,
        orbit_id: &str,
        mod_path: &str,
    ) -> Result<Option<ResolvedModule>> {
        let orbit_id = orbit_id.to_string();
        let mod_path = mod_path.to_string();
        self.run_store_blocking(move |store| store.resolve_module(&orbit_id, &mod_path))
            .await
    }

    pub async fn orbit_dir_async(&self, orbit_id: &str) -> Result<PathBuf> {
        self.ensure_orbit_dir_async(orbit_id).await
    }

    pub async fn list_orbit_files_async(&self, orbit_id: &str) -> Result<Vec<OrbitFileEntry>> {
        let orbit_id = orbit_id.to_string();
        self.run_store_blocking(move |store| store.list_orbit_files(&orbit_id))
            .await
    }

    pub async fn read_orbit_file_text_async(&self, orbit_id: &str, path: &str) -> Result<String> {
        let rel = validate_readable_orbit_path(path)?;
        let root = self.ensure_orbit_dir_async(orbit_id).await?;
        let path = root.join(rel);
        let root_canon = tokio::fs::canonicalize(&root).await?;
        let resolved = tokio::fs::canonicalize(&path).await?;
        if !resolved.starts_with(&root_canon) {
            bail!("arkorbit: resolved path escapes root");
        }
        Ok(tokio::fs::read_to_string(resolved).await?)
    }

    pub async fn write_orbit_file(&self, orbit_id: &str, path: &str, content: &str) -> Result<()> {
        self.store
            .write_orbit_file_async(orbit_id, path, content.as_bytes())
            .await
    }

    pub async fn remove_orbit_module_dir_async(
        &self,
        orbit_id: &str,
        module_name: &str,
    ) -> Result<bool> {
        let orbit_id = orbit_id.to_string();
        let module_name = module_name.to_string();
        self.run_store_blocking(move |store| store.remove_orbit_module_dir(&orbit_id, &module_name))
            .await
    }

    fn messages_path(&self, orbit_id: &str) -> Result<std::path::PathBuf> {
        Ok(self
            .store
            .ensure_orbit_dir(orbit_id)?
            .join("messages.jsonl"))
    }

    fn chat_history_dir(&self, orbit_id: &str) -> Result<std::path::PathBuf> {
        Ok(self
            .store
            .ensure_orbit_dir(orbit_id)?
            .join("data")
            .join("chat-history"))
    }

    async fn messages_path_async(&self, orbit_id: &str) -> Result<PathBuf> {
        Ok(self
            .ensure_orbit_dir_async(orbit_id)
            .await?
            .join("messages.jsonl"))
    }

    async fn chat_history_dir_async(&self, orbit_id: &str) -> Result<PathBuf> {
        Ok(self
            .ensure_orbit_dir_async(orbit_id)
            .await?
            .join("data")
            .join("chat-history"))
    }

    async fn chat_session_path_async(&self, orbit_id: &str) -> Result<PathBuf> {
        Ok(self
            .ensure_orbit_dir_async(orbit_id)
            .await?
            .join("data")
            .join("chat-session.txt"))
    }

    pub async fn ensure_orbit_chat_session_async(&self, orbit_id: &str) -> Result<String> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.chat_session_path_async(orbit_id).await?;
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => {
                let session_id = raw.trim();
                if Uuid::parse_str(session_id).is_ok() {
                    Ok(session_id.to_string())
                } else {
                    self.rotate_orbit_chat_session_async(orbit_id).await
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.rotate_orbit_chat_session_async(orbit_id).await
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn rotate_orbit_chat_session_async(&self, orbit_id: &str) -> Result<String> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.chat_session_path_async(orbit_id).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let session_id = Uuid::new_v4().to_string();
        tokio::fs::write(path, &session_id).await?;
        Ok(session_id)
    }

    pub async fn orbit_chat_session_matches_async(
        &self,
        orbit_id: &str,
        expected: &str,
    ) -> Result<bool> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.chat_session_path_async(orbit_id).await?;
        match tokio::fs::read_to_string(path).await {
            Ok(raw) => Ok(raw.trim() == expected),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn parse_chat_messages(raw: &str) -> Vec<OrbitChatMessage> {
        let mut messages = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<OrbitChatMessage>(line) {
                Ok(message) => messages.push(message),
                Err(error) => tracing::warn!(
                    target: "arkorbit.chat",
                    error = %error,
                    "Skipping malformed orbit chat line"
                ),
            }
        }
        messages
    }

    fn read_chat_messages_from_path(path: &Path) -> Result<Vec<OrbitChatMessage>> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        Ok(Self::parse_chat_messages(&raw))
    }

    async fn read_chat_messages_from_path_async(path: &Path) -> Result<Vec<OrbitChatMessage>> {
        let raw = match tokio::fs::read_to_string(path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        Ok(Self::parse_chat_messages(&raw))
    }

    fn summarize_chat_transcript(
        id: String,
        current: bool,
        messages: Vec<OrbitChatMessage>,
    ) -> Option<OrbitChatTranscriptSummary> {
        if messages.is_empty() {
            return None;
        }
        let created_at = messages
            .first()
            .map(|message| message.created_at.clone())
            .unwrap_or_else(Self::now);
        let updated_at = messages
            .last()
            .map(|message| message.created_at.clone())
            .unwrap_or_else(|| created_at.clone());
        let title = messages
            .iter()
            .find(|message| message.role == "user" && !message.content.trim().is_empty())
            .or_else(|| {
                messages
                    .iter()
                    .find(|message| !message.content.trim().is_empty())
            })
            .map(|message| {
                let mut text = message
                    .content
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.len() > 64 {
                    text.truncate(61);
                    text.push_str("...");
                }
                text
            })
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "Untitled chat".to_string());
        Some(OrbitChatTranscriptSummary {
            id,
            title,
            created_at,
            updated_at,
            message_count: messages.len(),
            current,
        })
    }

    pub fn read_orbit_chat_messages(
        &self,
        orbit_id: &str,
        limit: usize,
    ) -> Result<Vec<OrbitChatMessage>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.messages_path(orbit_id)?;
        let messages = Self::read_chat_messages_from_path(&path)?;
        let keep_from = messages.len().saturating_sub(limit.max(1));
        Ok(messages.into_iter().skip(keep_from).collect())
    }

    pub async fn read_orbit_chat_messages_async(
        &self,
        orbit_id: &str,
        limit: usize,
    ) -> Result<Vec<OrbitChatMessage>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.messages_path_async(orbit_id).await?;
        let messages = Self::read_chat_messages_from_path_async(&path).await?;
        let keep_from = messages.len().saturating_sub(limit.max(1));
        Ok(messages.into_iter().skip(keep_from).collect())
    }

    pub fn list_orbit_chat_transcripts(
        &self,
        orbit_id: &str,
    ) -> Result<Vec<OrbitChatTranscriptSummary>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let mut summaries = Vec::new();
        let current = self.read_orbit_chat_messages(orbit_id, usize::MAX)?;
        if let Some(summary) = Self::summarize_chat_transcript("current".to_string(), true, current)
        {
            summaries.push(summary);
        }

        let history_dir = self.chat_history_dir(orbit_id)?;
        let entries = match std::fs::read_dir(&history_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(summaries),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|value| value.to_string()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".jsonl").map(|value| value.to_string()) else {
                continue;
            };
            if !is_valid_chat_transcript_id(&id) {
                continue;
            }
            let messages = Self::read_chat_messages_from_path(&entry.path())?;
            if let Some(summary) = Self::summarize_chat_transcript(id, false, messages) {
                summaries.push(summary);
            }
        }
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    pub async fn list_orbit_chat_transcripts_async(
        &self,
        orbit_id: &str,
    ) -> Result<Vec<OrbitChatTranscriptSummary>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let mut summaries = Vec::new();
        let current = self
            .read_orbit_chat_messages_async(orbit_id, usize::MAX)
            .await?;
        if let Some(summary) = Self::summarize_chat_transcript("current".to_string(), true, current)
        {
            summaries.push(summary);
        }

        let history_dir = self.chat_history_dir_async(orbit_id).await?;
        let mut entries = match tokio::fs::read_dir(&history_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(summaries),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|value| value.to_string()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".jsonl").map(|value| value.to_string()) else {
                continue;
            };
            if !is_valid_chat_transcript_id(&id) {
                continue;
            }
            let messages = Self::read_chat_messages_from_path_async(&entry.path()).await?;
            if let Some(summary) = Self::summarize_chat_transcript(id, false, messages) {
                summaries.push(summary);
            }
        }
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    pub fn read_orbit_chat_transcript(
        &self,
        orbit_id: &str,
        transcript_id: &str,
        limit: usize,
    ) -> Result<Vec<OrbitChatMessage>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let messages = if transcript_id == "current" {
            self.read_orbit_chat_messages(orbit_id, usize::MAX)?
        } else {
            if !is_valid_chat_transcript_id(transcript_id) {
                bail!("ArkOrbit: invalid chat transcript id");
            }
            let path = self
                .chat_history_dir(orbit_id)?
                .join(format!("{}.jsonl", transcript_id));
            Self::read_chat_messages_from_path(&path)?
        };
        let keep_from = messages.len().saturating_sub(limit.max(1));
        Ok(messages.into_iter().skip(keep_from).collect())
    }

    pub async fn read_orbit_chat_transcript_async(
        &self,
        orbit_id: &str,
        transcript_id: &str,
        limit: usize,
    ) -> Result<Vec<OrbitChatMessage>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let messages = if transcript_id == "current" {
            self.read_orbit_chat_messages_async(orbit_id, usize::MAX)
                .await?
        } else {
            if !is_valid_chat_transcript_id(transcript_id) {
                bail!("ArkOrbit: invalid chat transcript id");
            }
            let path = self
                .chat_history_dir_async(orbit_id)
                .await?
                .join(format!("{}.jsonl", transcript_id));
            Self::read_chat_messages_from_path_async(&path).await?
        };
        let keep_from = messages.len().saturating_sub(limit.max(1));
        Ok(messages.into_iter().skip(keep_from).collect())
    }

    pub async fn reset_orbit_chat_async(
        &self,
        orbit_id: &str,
    ) -> Result<Option<OrbitChatTranscriptSummary>> {
        LayeredStore::validate_orbit_id(orbit_id)?;
        let path = self.messages_path_async(orbit_id).await?;
        let messages = Self::read_chat_messages_from_path_async(&path).await?;
        let orbit_dir = self.ensure_orbit_dir_async(orbit_id).await?;
        let summary_path = orbit_dir.join("data").join("chat-summary.md");
        if messages.is_empty() {
            remove_file_if_exists(&path).await?;
            remove_file_if_exists(&summary_path).await?;
            self.rotate_orbit_chat_session_async(orbit_id).await?;
            return Ok(None);
        }
        let history_dir = self.chat_history_dir_async(orbit_id).await?;
        tokio::fs::create_dir_all(&history_dir).await?;
        let id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            Uuid::new_v4().simple()
        );
        let archive_path = history_dir.join(format!("{}.jsonl", id));
        tokio::fs::rename(&path, &archive_path).await?;
        remove_file_if_exists(&summary_path).await?;
        self.rotate_orbit_chat_session_async(orbit_id).await?;
        Ok(Self::summarize_chat_transcript(id, false, messages))
    }

    pub async fn reconcile_filesystem(&self) -> Result<()> {
        tokio::fs::create_dir_all(self.store.orbits_root()).await?;
        Ok(())
    }
}

async fn remove_file_if_exists(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn is_valid_chat_transcript_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service(data_dir: &Path) -> ArkOrbitService {
        ArkOrbitService {
            store: Arc::new(LayeredStore::new(data_dir)),
        }
    }

    #[tokio::test]
    async fn async_chat_session_round_trips_and_rotates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = test_service(dir.path());
        let orbit_id = Uuid::new_v4().to_string();

        let first = service
            .ensure_orbit_chat_session_async(&orbit_id)
            .await
            .expect("first session");
        let second = service
            .ensure_orbit_chat_session_async(&orbit_id)
            .await
            .expect("second session");
        let rotated = service
            .rotate_orbit_chat_session_async(&orbit_id)
            .await
            .expect("rotated session");

        assert_eq!(first, second);
        assert_ne!(first, rotated);
        assert!(service
            .orbit_chat_session_matches_async(&orbit_id, &rotated)
            .await
            .expect("session matches"));
    }

    #[tokio::test]
    async fn async_chat_history_reads_recent_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = test_service(dir.path());
        let orbit_id = Uuid::new_v4().to_string();
        let path = service
            .messages_path_async(&orbit_id)
            .await
            .expect("messages path");
        let first = OrbitChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: "first".to_string(),
            created_at: "2026-05-28T00:00:00Z".to_string(),
            status: None,
            activity: None,
            model: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost_usd: None,
            estimated: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        };
        let second = OrbitChatMessage {
            content: "second".to_string(),
            created_at: "2026-05-28T00:01:00Z".to_string(),
            ..first.clone()
        };
        let lines = format!(
            "{}\n{}\n",
            serde_json::to_string(&first).expect("first json"),
            serde_json::to_string(&second).expect("second json")
        );
        tokio::fs::write(path, lines).await.expect("write messages");

        let recent = service
            .read_orbit_chat_messages_async(&orbit_id, 1)
            .await
            .expect("recent messages");

        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "second");
    }
}
