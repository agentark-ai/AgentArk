//! Cognitive memory retrieval for episodic entries and learned experience items.
//!
//! Inspired by human memory systems and recent research:
//! - arXiv:2512.13564 "Memory in the Age of AI Agents"
//! - arXiv:2601.01885 "Agentic Memory (AgeMem)"
//! - Park et al. "Generative Agents" (2023) - Memory decay and retrieval scoring

use anyhow::Result;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use sea_orm::entity::prelude::PgVector;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use crate::core::embeddings::EmbeddingClient;
use crate::storage::Storage;

/// Memory decay configuration
/// Based on Generative Agents: final_score = α*relevance + β*recency + γ*importance
#[derive(Debug, Clone)]
pub struct MemoryDecayConfig {
    /// Weight for relevance/similarity score (α)
    pub relevance_weight: f32,
    /// Weight for recency score (β)
    pub recency_weight: f32,
    /// Weight for importance score (γ)
    pub importance_weight: f32,
    /// Daily decay rate (λ) - higher = faster day-scale decay
    /// recency = exp(-λ * hours_since_creation / 24)
    pub decay_rate: f32,
    /// Bonus for recently accessed memories
    pub access_recency_bonus: f32,
}

impl Default for MemoryDecayConfig {
    fn default() -> Self {
        Self {
            relevance_weight: 1.0,
            recency_weight: 1.0,
            importance_weight: 1.0,
            decay_rate: 0.099, // 50% decay per week (ln(2)/7), memories useful for ~30 days
            access_recency_bonus: 0.1,
        }
    }
}

/// Context for an episodic memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeContext {
    pub channel: String,
    pub timestamp: DateTime<Utc>,
    pub location: Option<String>,
    pub participants: Vec<String>,
    pub project_id: Option<String>,
}

/// A memory entry (can be episodic or semantic)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub content: String,
    pub memory_type: MemoryType,
    pub timestamp: DateTime<Utc>,
    /// Semantic similarity to current query (0.0-1.0)
    pub relevance_score: f32,
    /// User/LLM-assigned importance (0.0-1.0)
    pub importance: f32,
    /// Time-decayed recency score (0.0-1.0)
    pub recency_score: f32,
    /// Final combined score used for ranking
    pub final_score: f32,
    /// Number of times this memory was accessed
    pub access_count: i32,
}

/// Type of memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryType {
    /// Specific experiences with context
    Episodic { context: EpisodeContext },
    /// Generalized facts/knowledge
    Semantic { confidence: f32, sources: Vec<Uuid> },
    /// Learned actions/procedures
    Procedural {
        action_name: String,
        success_rate: f32,
    },
}

/// Cognitive memory system managing all memory types
pub struct CognitiveMemory {
    storage: Arc<Storage>,
    /// Encrypted storage for sensitive content (episodes, facts)
    encrypted_storage: crate::storage::encrypted::EncryptedStorage,
    embedding_client: RwLock<Option<Arc<EmbeddingClient>>>,
    episode_count: AtomicUsize,
    /// Configuration for memory decay and scoring
    decay_config: MemoryDecayConfig,
}

#[cfg(test)]
fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated.trim_end())
    } else {
        truncated
    }
}

const MEMORY_VECTOR_SHORTLIST_MULTIPLIER: usize = 12;
const MEMORY_VECTOR_SHORTLIST_MIN: usize = 96;
const MEMORY_VECTOR_SHORTLIST_MAX: usize = 384;
const MEMORY_RECENT_SHORTLIST_MULTIPLIER: usize = 4;
const MEMORY_RECENT_SHORTLIST_MIN: usize = 32;
const MEMORY_RECENT_SHORTLIST_MAX: usize = 128;

fn shortlist_limit(multiplier: usize, min: usize, max: usize, limit: usize) -> u64 {
    let bounded = limit.max(1).saturating_mul(multiplier).clamp(min, max);
    bounded as u64
}

fn merge_candidate_ids(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(primary.len() + secondary.len());
    for id in primary.into_iter().chain(secondary) {
        if seen.insert(id.clone()) {
            merged.push(id);
        }
    }
    merged
}

impl CognitiveMemory {
    pub async fn new(
        _data_dir: &Path,
        storage: Storage,
        encrypted_storage: crate::storage::encrypted::EncryptedStorage,
        embedding_client: Option<Arc<EmbeddingClient>>,
    ) -> Result<Self> {
        Self::with_config(
            _data_dir,
            storage,
            encrypted_storage,
            embedding_client,
            MemoryDecayConfig::default(),
        )
        .await
    }

    pub async fn with_config(
        _data_dir: &Path,
        storage: Storage,
        encrypted_storage: crate::storage::encrypted::EncryptedStorage,
        embedding_client: Option<Arc<EmbeddingClient>>,
        decay_config: MemoryDecayConfig,
    ) -> Result<Self> {
        let storage = Arc::new(storage);

        // Count existing episodes
        let episode_count = storage.count_episodes().await.unwrap_or(0) as usize;

        Ok(Self {
            storage,
            encrypted_storage,
            embedding_client: RwLock::new(embedding_client),
            episode_count: AtomicUsize::new(episode_count),
            decay_config,
        })
    }

    pub fn set_embedding_client(&self, embedding_client: Option<Arc<EmbeddingClient>>) {
        *self.embedding_client.write() = embedding_client;
    }

    /// Calculate recency score using exponential decay
    /// recency = exp(-λ * hours_since_creation / 24)
    fn calculate_recency_score(
        &self,
        timestamp: DateTime<Utc>,
        last_accessed: Option<DateTime<Utc>>,
    ) -> f32 {
        let now = Utc::now();
        let hours_since_creation = (now - timestamp).num_hours() as f32;

        // Base recency from creation time
        let base_recency = (-self.decay_config.decay_rate * hours_since_creation / 24.0).exp();

        // Bonus if recently accessed
        let access_bonus = if let Some(last_access) = last_accessed {
            let hours_since_access = (now - last_access).num_hours() as f32;
            let access_recency = (-self.decay_config.decay_rate * hours_since_access / 24.0).exp();
            access_recency * self.decay_config.access_recency_bonus
        } else {
            0.0
        };

        (base_recency + access_bonus).min(1.0)
    }

    /// Calculate final memory score using weighted combination
    /// final_score = α*relevance + β*recency + γ*importance
    fn calculate_final_score(&self, relevance: f32, recency: f32, importance: f32) -> f32 {
        let config = &self.decay_config;

        // Normalize weights
        let total_weight =
            config.relevance_weight + config.recency_weight + config.importance_weight;

        if total_weight == 0.0 {
            return 0.0;
        }

        let normalized_relevance = config.relevance_weight / total_weight;
        let normalized_recency = config.recency_weight / total_weight;
        let normalized_importance = config.importance_weight / total_weight;

        normalized_relevance * relevance
            + normalized_recency * recency
            + normalized_importance * importance
    }

    /// Calculate simple fallback relevance score based on word overlap.
    fn calculate_relevance(&self, query: &str, content: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let content_lower = content.to_lowercase();

        let query_words: std::collections::HashSet<&str> = query_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        let content_words: std::collections::HashSet<&str> = content_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        if query_words.is_empty() || content_words.is_empty() {
            return 0.0;
        }

        let intersection = query_words.intersection(&content_words).count();
        let query_coverage = intersection as f32 / query_words.len() as f32;

        let phrase_boost = if content_lower.contains(&query_lower) {
            0.3
        } else {
            0.0
        };

        (query_coverage + phrase_boost).min(1.0)
    }

    async fn embed_text(&self, text: &str) -> Option<PgVector> {
        let Some(client) = self.embedding_client.read().clone() else {
            tracing::warn!(
                "Memory embedding skipped because no embedding client is configured; using lexical memory fallback"
            );
            return None;
        };
        match client.embed_texts(&[text.to_string()]).await {
            Ok(values) => values.into_iter().next(),
            Err(error) => {
                tracing::warn!(
                    "Memory embedding failed; using lexical memory fallback until the embedding backend recovers: {}",
                    error
                );
                None
            }
        }
    }

    fn dense_similarity(
        query_embedding: Option<&PgVector>,
        candidate_embedding: Option<&PgVector>,
    ) -> Option<f32> {
        crate::core::document_search::normalized_embedding_similarity(
            query_embedding?.as_slice(),
            candidate_embedding?.as_slice(),
        )
        .map(|score| score.clamp(0.0, 1.0))
    }

    /// Add an episodic memory (encrypted at rest).
    pub async fn add_episode(
        &self,
        content: String,
        context: EpisodeContext,
        importance: f32,
        project_id: Option<&str>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let context_json = serde_json::to_string(&context)?;
        let bounded_importance = importance.clamp(0.0, 1.0);
        let embedding = self.embed_text(&content).await;

        self.encrypted_storage
            .insert_episode_encrypted(
                &id.to_string(),
                &content,
                &context_json,
                embedding,
                bounded_importance,
                project_id,
            )
            .await?;

        self.episode_count.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    async fn load_episode_candidates(
        &self,
        query_embedding: Option<&PgVector>,
        project_id: Option<&str>,
        limit: usize,
    ) -> Vec<crate::storage::entities::episode::Model> {
        let load_fallback = || async {
            if project_id.is_some() {
                match self
                    .encrypted_storage
                    .get_all_episodes_for_scoring_by_project_decrypted(project_id)
                    .await
                {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!(
                            project_id = ?project_id,
                            "Episode lexical fallback load failed: {}",
                            error
                        );
                        Vec::new()
                    }
                }
            } else {
                match self
                    .encrypted_storage
                    .get_all_episodes_for_scoring_decrypted()
                    .await
                {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!("Episode lexical fallback load failed: {}", error);
                        Vec::new()
                    }
                }
            }
        };

        let Some(query_embedding) = query_embedding else {
            tracing::warn!(
                "Dense memory query embedding unavailable; using lexical memory fallback"
            );
            return load_fallback().await;
        };

        let dense_ids = match self
            .storage
            .nearest_episode_ids(
                query_embedding,
                project_id,
                shortlist_limit(
                    MEMORY_VECTOR_SHORTLIST_MULTIPLIER,
                    MEMORY_VECTOR_SHORTLIST_MIN,
                    MEMORY_VECTOR_SHORTLIST_MAX,
                    limit,
                ),
            )
            .await
        {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(
                    project_id = ?project_id,
                    "Episode pgvector shortlist failed: {}",
                    error
                );
                Vec::new()
            }
        };
        let recent_ids = match self
            .storage
            .list_recent_episode_ids_for_scoring(
                project_id,
                shortlist_limit(
                    MEMORY_RECENT_SHORTLIST_MULTIPLIER,
                    MEMORY_RECENT_SHORTLIST_MIN,
                    MEMORY_RECENT_SHORTLIST_MAX,
                    limit,
                ),
            )
            .await
        {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(
                    project_id = ?project_id,
                    "Episode recent shortlist failed: {}",
                    error
                );
                Vec::new()
            }
        };
        let candidate_ids = merge_candidate_ids(dense_ids, recent_ids);

        if candidate_ids.is_empty() {
            tracing::warn!("Episode shortlist was empty; using lexical memory fallback");
            load_fallback().await
        } else {
            match self
                .encrypted_storage
                .get_episodes_by_ids_decrypted(&candidate_ids)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        project_id = ?project_id,
                        ids = candidate_ids.len(),
                        "Episode shortlist hydrate failed; using lexical memory fallback: {}",
                        error
                    );
                    load_fallback().await
                }
            }
        }
    }

    /// Retrieve relevant memories for a query using decay-based scoring
    /// Implements: final_score = α*relevance + β*recency + γ*importance
    pub async fn retrieve_relevant(
        &self,
        query: &str,
        limit: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let query_embedding = self.embed_text(query).await;

        let episodes = self
            .load_episode_candidates(query_embedding.as_ref(), project_id, limit)
            .await;
        let mut entries: Vec<MemoryEntry> = episodes
            .into_iter()
            .map(|e| {
                let context: EpisodeContext =
                    serde_json::from_str(&e.context).unwrap_or(EpisodeContext {
                        channel: "unknown".to_string(),
                        timestamp: Utc::now(),
                        location: None,
                        participants: vec![],
                        project_id: None,
                    });

                let timestamp = chrono::DateTime::parse_from_rfc3339(&e.timestamp)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                let last_accessed = e.last_accessed.as_ref().and_then(|la| {
                    chrono::DateTime::parse_from_rfc3339(la)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });

                let lexical_relevance = self.calculate_relevance(query, &e.content);
                let dense_relevance =
                    Self::dense_similarity(query_embedding.as_ref(), e.embedding.as_ref())
                        .unwrap_or(0.0);
                let relevance_score = lexical_relevance.max(dense_relevance);
                let recency_score = self.calculate_recency_score(timestamp, last_accessed);
                let importance = e.importance;

                // Calculate final weighted score
                let final_score =
                    self.calculate_final_score(relevance_score, recency_score, importance);

                MemoryEntry {
                    id: Uuid::parse_str(&e.id).unwrap_or_else(|_| Uuid::new_v4()),
                    content: e.content,
                    memory_type: MemoryType::Episodic { context },
                    timestamp,
                    relevance_score,
                    importance,
                    recency_score,
                    final_score,
                    access_count: e.access_count,
                }
            })
            .collect();

        // Sort by final score (highest first)
        entries.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Keep only memories with at least minimal relevance so pure recency
        // does not surface unrelated context.
        let top_entries: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| e.relevance_score >= 0.08)
            .take(limit)
            .collect();

        // Update access times for retrieved memories (async, fire-and-forget)
        for entry in &top_entries {
            let _ = self.storage.touch_episode(&entry.id.to_string()).await;
        }

        Ok(top_entries)
    }

    /// Get total entry count
    pub fn entry_count(&self) -> usize {
        self.episode_count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_chars;

    #[test]
    fn truncate_chars_preserves_utf8_boundaries() {
        let input = "Search results: Reuters › World › Iran";
        let output = truncate_chars(input, 26);
        assert_eq!(output, "Search results: Reuters ›...");
    }
}
