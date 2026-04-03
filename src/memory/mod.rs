//! Cognitive Memory System - Episodic, Semantic, and Procedural Memory
//!
//! Inspired by human memory systems and recent research:
//! - arXiv:2512.13564 "Memory in the Age of AI Agents"
//! - arXiv:2601.01885 "Agentic Memory (AgeMem)"
//! - Park et al. "Generative Agents" (2023) - Memory decay and retrieval scoring

use anyhow::Result;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use sea_orm::entity::prelude::PgVector;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
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
const MEMORY_CONSOLIDATION_LEASE_KEY: &str = "memory_llm_consolidation_lease_v1";
const MEMORY_CONSOLIDATION_LEASE_TTL_SECS: i64 = 30 * 60;
static MEMORY_CONSOLIDATION_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn shortlist_limit(multiplier: usize, min: usize, max: usize, limit: usize) -> u64 {
    let bounded = limit
        .max(1)
        .saturating_mul(multiplier)
        .clamp(min, max);
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

    /// Calculate simple relevance score based on word overlap
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

        // Boost for exact phrase matches
        let phrase_boost = if content_lower.contains(&query_lower) {
            0.3
        } else {
            0.0
        };

        (query_coverage + phrase_boost).min(1.0)
    }

    async fn embed_text(&self, text: &str) -> Option<PgVector> {
        let client = self.embedding_client.read().clone()?;
        let values = client.embed_texts(&[text.to_string()]).await.ok()?;
        values.into_iter().next()
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

    /// Add a semantic fact
    pub async fn add_fact(
        &self,
        fact: String,
        confidence: f32,
        sources: Vec<Uuid>,
        project_id: Option<&str>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let embedding = self.embed_text(&fact).await;
        let sources_json = serde_json::to_string(&sources)?;

        self.encrypted_storage
            .insert_fact_encrypted(
                &id.to_string(),
                &fact,
                confidence,
                &sources_json,
                embedding,
                project_id,
            )
            .await?;

        Ok(id)
    }

    async fn load_semantic_facts_for_scope(
        &self,
        project_id: Option<&str>,
    ) -> Vec<crate::storage::entities::semantic_fact::Model> {
        if let Some(project_id) = project_id {
            let scoped_count = match self.encrypted_storage.count_facts(Some(project_id)).await {
                Ok(count) => count,
                Err(error) => {
                    tracing::warn!(
                        project_id = project_id,
                        "Scoped fact count failed: {}",
                        error
                    );
                    0
                }
            };
            let mut merged = match self
                .encrypted_storage
                .get_facts_by_project_decrypted(scoped_count, 0, Some(project_id))
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        project_id = project_id,
                        "Scoped fact fallback load failed: {}",
                        error
                    );
                    Vec::new()
                }
            };
            let global_count = match self.encrypted_storage.count_global_facts().await {
                Ok(count) => count,
                Err(error) => {
                    tracing::warn!("Global fact count failed: {}", error);
                    0
                }
            };
            let global = match self
                .encrypted_storage
                .get_global_facts_decrypted(global_count, 0)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!("Global fact fallback load failed: {}", error);
                    Vec::new()
                }
            };
            let mut seen_ids = merged
                .iter()
                .map(|fact| fact.id.clone())
                .collect::<std::collections::HashSet<_>>();
            for fact in global {
                if seen_ids.insert(fact.id.clone()) {
                    merged.push(fact);
                }
            }
              merged
          } else {
            match self.encrypted_storage.get_facts_decrypted().await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!("Fact fallback load failed: {}", error);
                    Vec::new()
                }
            }
        }
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
                            "Episode fallback load failed: {}",
                            error
                        );
                        Vec::new()
                    }
                }
            } else {
                match self.encrypted_storage.get_all_episodes_for_scoring_decrypted().await {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!("Episode fallback load failed: {}", error);
                        Vec::new()
                    }
                }
            }
        };

        let Some(query_embedding) = query_embedding else {
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
                        "Episode shortlist hydrate failed: {}",
                        error
                    );
                    load_fallback().await
                }
            }
        }
    }

    async fn load_semantic_fact_candidates(
        &self,
        query_embedding: Option<&PgVector>,
        project_id: Option<&str>,
        limit: usize,
    ) -> Vec<crate::storage::entities::semantic_fact::Model> {
        let load_fallback = || async {
            let rows = self.load_semantic_facts_for_scope(project_id).await;
            if rows.is_empty() {
                tracing::debug!(project_id = ?project_id, "Semantic fact fallback returned no rows");
            }
            rows
        };

        let Some(query_embedding) = query_embedding else {
            return load_fallback().await;
        };

        let dense_ids = match self
            .storage
            .nearest_semantic_fact_ids(
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
                    "Semantic fact pgvector shortlist failed: {}",
                    error
                );
                Vec::new()
            }
        };
        let recent_ids = match self
            .storage
            .list_recent_fact_ids_for_scope(
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
                    "Semantic fact recent shortlist failed: {}",
                    error
                );
                Vec::new()
            }
        };
        let candidate_ids = merge_candidate_ids(dense_ids, recent_ids);

        if candidate_ids.is_empty() {
            load_fallback().await
        } else {
            match self
                .encrypted_storage
                .get_facts_by_ids_decrypted(&candidate_ids)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        project_id = ?project_id,
                        ids = candidate_ids.len(),
                        "Semantic fact shortlist hydrate failed: {}",
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
        let facts = self
            .load_semantic_fact_candidates(query_embedding.as_ref(), project_id, limit)
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

                // Calculate scores
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

        entries.extend(facts.into_iter().map(|fact| {
            let timestamp = chrono::DateTime::parse_from_rfc3339(&fact.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let lexical_relevance = self.calculate_relevance(query, &fact.fact);
            let dense_relevance =
                Self::dense_similarity(query_embedding.as_ref(), fact.embedding.as_ref())
                    .unwrap_or(0.0);
            let relevance_score = lexical_relevance.max(dense_relevance);
            let recency_score = self.calculate_recency_score(timestamp, None);
            let importance = fact.confidence.clamp(0.0, 1.0);
            let final_score =
                self.calculate_final_score(relevance_score, recency_score, importance);

            MemoryEntry {
                id: Uuid::parse_str(&fact.id).unwrap_or_else(|_| Uuid::new_v4()),
                content: fact.fact,
                memory_type: MemoryType::Semantic {
                    confidence: fact.confidence,
                    sources: serde_json::from_str(&fact.sources).unwrap_or_default(),
                },
                timestamp,
                relevance_score,
                importance,
                recency_score,
                final_score,
                access_count: 0,
            }
        }));

        // Sort by final score (highest first)
        entries.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Keep only memories with at least minimal lexical relevance so pure recency
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

    /// Check if a new fact is too similar to any existing fact (deduplication)
    /// Returns true if a duplicate/near-duplicate exists
    async fn is_duplicate_fact(&self, new_fact: &str, project_id: Option<&str>) -> bool {
        let existing = self.load_semantic_facts_for_scope(project_id).await;
        if existing.is_empty() {
            return false;
        }

        let new_lower = new_fact.to_lowercase();
        let new_words: std::collections::HashSet<&str> = new_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        for fact in &existing {
            let existing_lower = fact.fact.to_lowercase();

            // Exact or near-exact match
            if existing_lower == new_lower {
                return true;
            }

            // Substring containment (one contains the other)
            if existing_lower.contains(&new_lower) || new_lower.contains(&existing_lower) {
                return true;
            }

            // High word overlap (Jaccard > 0.7)
            let existing_words: std::collections::HashSet<&str> = existing_lower
                .split_whitespace()
                .filter(|w| w.len() > 2)
                .collect();

            if !new_words.is_empty() && !existing_words.is_empty() {
                let intersection = new_words.intersection(&existing_words).count();
                let union = new_words.union(&existing_words).count();
                let jaccard = intersection as f32 / union as f32;
                if jaccard > 0.7 {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a fact passes minimum quality bar for long-term storage
    fn is_quality_fact(fact_text: &str) -> bool {
        let text = fact_text.trim();

        // Too short to be useful
        if text.len() < 15 {
            return false;
        }

        // Reject facts that are just describing a request/action (not durable knowledge)
        let transient_patterns = [
            "user requested",
            "user asked",
            "user wants to",
            "user said",
            "was requested",
            "was asked",
            "tried to",
            "attempted to",
            "failed to",
            "error occurred",
            "error when",
            "socket not found",
            "connection failed",
            "docker socket",
            "execution failed",
            "generated a",
            "created a",
            "built a",
            "ran a",
            "the conversation",
            "in this session",
            "during the chat",
            "the system",
            "the agent",
            "the bot",
        ];
        let lower = text.to_lowercase();
        for pattern in &transient_patterns {
            if lower.contains(pattern) {
                return false;
            }
        }

        // Reject if it's mostly about a single transient event
        let event_starters = [
            "a qr code",
            "a code was",
            "code was executed",
            "the code",
            "an image was",
            "a file was",
            "output was",
        ];
        for starter in &event_starters {
            if lower.starts_with(starter) {
                return false;
            }
        }

        true
    }

    /// LLM-powered memory consolidation
    /// Groups unconsolidated episodes, sends batches to LLM for summarization/dedup,
    /// stores consolidated facts, and marks episodes as processed.
    pub async fn run_llm_consolidation(&self, llm: &crate::core::LlmClient) -> Result<String> {
        let Ok(_guard) = MEMORY_CONSOLIDATION_LOCK.try_lock() else {
            return Ok("Memory consolidation already in progress.".to_string());
        };
        let lease_owner = Uuid::new_v4().to_string();
        match self
            .storage
            .acquire_kv_lease(
                MEMORY_CONSOLIDATION_LEASE_KEY,
                &lease_owner,
                MEMORY_CONSOLIDATION_LEASE_TTL_SECS,
            )
            .await?
        {
            true => {}
            false => {
                return Ok("Memory consolidation already in progress on another worker.".to_string())
            }
        }
        let heartbeat_storage = self.storage.clone();
        let heartbeat_owner = lease_owner.clone();
        let (lease_stop_tx, mut lease_stop_rx) = tokio::sync::watch::channel(false);
        let lease_heartbeat = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = lease_stop_rx.changed() => {
                        if changed.is_err() || *lease_stop_rx.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                        match heartbeat_storage
                            .refresh_kv_lease(
                                MEMORY_CONSOLIDATION_LEASE_KEY,
                                &heartbeat_owner,
                                MEMORY_CONSOLIDATION_LEASE_TTL_SECS,
                            )
                            .await
                        {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!("Memory consolidation lease heartbeat lost ownership");
                                break;
                            }
                            Err(error) => {
                                tracing::warn!(
                                    "Memory consolidation lease heartbeat refresh failed: {}",
                                    error
                                );
                                break;
                            }
                        }
                    }
                }
            }
        });
        let result = async {
            let episodes = self
                .encrypted_storage
                .get_unconsolidated_episodes_decrypted(50)
                .await?;
            if episodes.is_empty() {
                return Ok("No unconsolidated episodes found.".to_string());
            }

        tracing::info!("LLM consolidation: processing {} episodes", episodes.len());

        // Load existing facts for dedup context (decrypted)
        let existing_facts: Vec<String> = self
            .encrypted_storage
            .get_facts_decrypted()
            .await
            .unwrap_or_default()
            .iter()
            .map(|f| f.fact.clone())
            .collect();

        let existing_facts_text = if existing_facts.is_empty() {
            "None yet.".to_string()
        } else {
            existing_facts
                .iter()
                .enumerate()
                .map(|(i, f)| format!("  {}. {}", i + 1, f))
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Group episodes into batches of ~10 for LLM processing
        let batch_size = 10;
        let mut total_facts = 0;
        let mut skipped_dupes = 0;
        let mut skipped_quality = 0;
        let mut consolidated_episode_count = 0;
        let mut fact_store_failures = 0;
        let mut summaries = Vec::new();

        for batch in episodes.chunks(batch_size) {
            match self
                .storage
                .refresh_kv_lease(
                    MEMORY_CONSOLIDATION_LEASE_KEY,
                    &lease_owner,
                    MEMORY_CONSOLIDATION_LEASE_TTL_SECS,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("Lost memory consolidation lease before batch execution");
                    summaries.push(
                        "Stopped remaining batches after losing the consolidation lease."
                            .to_string(),
                    );
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to refresh memory consolidation lease before batch: {}",
                        error
                    );
                    summaries.push(format!(
                        "Stopped remaining batches after lease refresh failure: {}",
                        error
                    ));
                    break;
                }
            }

            // Build a prompt with the batch of memories
            let memories_text: String = batch
                .iter()
                .enumerate()
                .map(|(i, ep)| {
                    let timestamp = &ep.timestamp;
                    let content = truncate_chars(&ep.content, 300);
                    format!("{}. [{}] {}", i + 1, timestamp, content)
                })
                .collect::<Vec<_>>()
                .join("\n");

            let consolidation_prompt = format!(
                r#"You are a STRICT memory consolidation engine for a personal AI assistant. Your job is to extract ONLY information worth remembering permanently about the USER — like how a close friend would naturally remember things about someone over time.

MEMORIES TO PROCESS:
{memories}

ALREADY STORED FACTS (do NOT duplicate these):
{existing}

WHAT TO EXTRACT (only if clearly present):
- PREF: User preferences, likes, dislikes, habits, communication style
  Example: "PREF: User prefers dark mode and minimal UIs"
  Example: "PREF: User communicates directly and dislikes verbose explanations"
- FACT: Durable personal facts about the user — name, job, skills, location, tools they use
  Example: "FACT: User is a backend developer working with Rust and Python"
  Example: "FACT: User's name is Alex and they work at a startup"
- PATTERN: Recurring behaviors or workflows the user consistently follows
  Example: "PATTERN: User typically tests features by asking the agent to generate QR codes"

WHAT TO REJECT (never extract these):
- Individual requests ("user asked to generate a QR code") — these are transient actions, not lasting knowledge
- System errors or technical failures ("Docker socket not found", "connection failed")
- Descriptions of what the agent/system did ("agent executed code", "response was generated")
- One-time events that don't reveal anything lasting about the user
- Anything already captured in the existing facts above (even if worded differently)
- Greetings, small talk, or conversational filler
- Facts about the AI system itself rather than the user

RULES:
- Be extremely selective — it's better to extract NOTHING than to store noise
- Each fact must be about the USER, not about a specific interaction
- Each fact must be useful weeks or months from now
- If the memories are routine interactions with no personal insight, respond with "NO_FACTS"
- Maximum 3 items per batch — quality over quantity

Output ONLY prefixed lines (FACT:/PATTERN:/PREF:) or "NO_FACTS". Nothing else."#,
                memories = memories_text,
                existing = existing_facts_text,
            );

            // Use simple chat (no tools needed)
            let mut should_mark_batch = false;
            match llm.chat(
                "You are a strict memory filter. Only extract genuinely useful, durable personal facts about the user. When in doubt, output NO_FACTS.",
                &consolidation_prompt,
                &[],
                &[],
            ).await {
                Ok(response) => {
                    let text = response.content.trim();
                    if text == "NO_FACTS" || text.is_empty() {
                        should_mark_batch = true;
                        summaries.push(format!("Batch: {} episodes -> no durable facts", batch.len()));
                    } else {
                        // Parse extracted facts
                        let mut batch_fact_store_failed = false;
                        for line in text.lines() {
                            let line = line.trim();
                            if line.is_empty() { continue; }

                            let (fact_text, confidence) = if line.starts_with("FACT: ") {
                                (line.strip_prefix("FACT: ").unwrap_or(line), 0.7)
                            } else if line.starts_with("PATTERN: ") {
                                (line.strip_prefix("PATTERN: ").unwrap_or(line), 0.8)
                            } else if line.starts_with("PREF: ") {
                                (line.strip_prefix("PREF: ").unwrap_or(line), 0.9)
                            } else {
                                continue; // Skip unformatted lines
                            };

                            // Quality gate: reject trivial/transient facts
                            if !Self::is_quality_fact(fact_text) {
                                tracing::debug!("Rejected low-quality fact ({} chars)", fact_text.len());
                                skipped_quality += 1;
                                continue;
                            }

                            // Dedup gate: reject if too similar to existing facts
                            if self.is_duplicate_fact(fact_text, None).await {
                                tracing::debug!("Rejected duplicate fact ({} chars)", fact_text.len());
                                skipped_dupes += 1;
                                continue;
                            }

                            // Store as semantic fact
                            let source_ids: Vec<uuid::Uuid> = batch.iter()
                                .filter_map(|ep| uuid::Uuid::parse_str(&ep.id).ok())
                                .collect();

                            if let Err(e) = self.add_fact(
                                fact_text.to_string(),
                                confidence,
                                source_ids,
                                None,
                            ).await {
                                tracing::warn!("Failed to store consolidated fact: {}", e);
                                batch_fact_store_failed = true;
                                fact_store_failures += 1;
                            } else {
                                total_facts += 1;
                            }
                        }

                        if batch_fact_store_failed {
                            summaries.push(format!(
                                "Batch deferred for retry after fact-write failure ({} episodes)",
                                batch.len()
                            ));
                        } else {
                            should_mark_batch = true;
                            summaries.push(format!(
                                "Batch: {} episodes -> {} lines",
                                batch.len(),
                                text.lines().count()
                            ));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("LLM consolidation batch failed: {}", e);
                    summaries.push(format!("Batch failed: {}", e));
                }
            }

            if should_mark_batch {
                let batch_ids = batch
                    .iter()
                    .map(|episode| episode.id.clone())
                    .collect::<Vec<_>>();
                match self.storage.mark_episodes_consolidated(&batch_ids).await {
                    Ok(()) => {
                        consolidated_episode_count += batch.len();
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Failed to mark episodes consolidated for batch: {}",
                            error
                        );
                        summaries.push(format!(
                            "Failed to mark {} episodes consolidated: {}",
                            batch.len(),
                            error
                        ));
                    }
                }
            }

            match self
                .storage
                .refresh_kv_lease(
                    MEMORY_CONSOLIDATION_LEASE_KEY,
                    &lease_owner,
                    MEMORY_CONSOLIDATION_LEASE_TTL_SECS,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("Lost memory consolidation lease after batch execution");
                    summaries.push(
                        "Stopped remaining batches after losing the consolidation lease."
                            .to_string(),
                    );
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to refresh memory consolidation lease after batch: {}",
                        error
                    );
                    summaries.push(format!(
                        "Stopped remaining batches after lease refresh failure: {}",
                        error
                    ));
                    break;
                }
            }
        }

            let summary = format!(
                "Consolidated {} episodes into {} facts (skipped {} dupes, {} low-quality, {} fact-write failures). {}",
                consolidated_episode_count,
                total_facts,
                skipped_dupes,
                skipped_quality,
                fact_store_failures,
                summaries.join(" | ")
            );
            tracing::info!("LLM consolidation complete: {}", summary);
            Ok(summary)
        }
        .await;

        let _ = lease_stop_tx.send(true);
        if let Err(error) = lease_heartbeat.await {
            tracing::warn!(
                "Failed to join memory consolidation lease heartbeat: {}",
                error
            );
        }

        if let Err(error) = self
            .storage
            .release_kv_lease(MEMORY_CONSOLIDATION_LEASE_KEY, &lease_owner)
            .await
        {
            tracing::warn!("Failed to release memory consolidation lease: {}", error);
        }

        result
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
