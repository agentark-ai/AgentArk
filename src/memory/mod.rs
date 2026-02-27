//! Cognitive Memory System - Episodic, Semantic, and Procedural Memory
//!
//! Inspired by human memory systems and recent research:
//! - arXiv:2512.13564 "Memory in the Age of AI Agents"
//! - arXiv:2601.01885 "Agentic Memory (AgeMem)"
//! - Park et al. "Generative Agents" (2023) - Memory decay and retrieval scoring

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

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
    /// Decay rate (λ) - higher = faster decay
    /// recency = exp(-λ * hours_since_creation)
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
            decay_rate: 0.995, // ~50% decay per day (24 hours)
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
    episode_count: AtomicUsize,
    /// Configuration for memory decay and scoring
    decay_config: MemoryDecayConfig,
}

impl CognitiveMemory {
    pub async fn new(
        _data_dir: &Path,
        storage: Storage,
        encrypted_storage: crate::storage::encrypted::EncryptedStorage,
    ) -> Result<Self> {
        Self::with_config(
            _data_dir,
            storage,
            encrypted_storage,
            MemoryDecayConfig::default(),
        )
        .await
    }

    pub async fn with_config(
        _data_dir: &Path,
        storage: Storage,
        encrypted_storage: crate::storage::encrypted::EncryptedStorage,
        decay_config: MemoryDecayConfig,
    ) -> Result<Self> {
        let storage = Arc::new(storage);

        // Count existing episodes
        let episode_count = storage.count_episodes().await.unwrap_or(0) as usize;

        Ok(Self {
            storage,
            encrypted_storage,
            episode_count: AtomicUsize::new(episode_count),
            decay_config,
        })
    }

    /// Calculate recency score using exponential decay
    /// recency = exp(-λ * hours_since_creation)
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

        self.encrypted_storage
            .insert_episode_encrypted(
                &id.to_string(),
                &content,
                &context_json,
                None,
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
        let embedding: Option<Vec<u8>> = None;
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

    /// Retrieve relevant memories for a query using decay-based scoring
    /// Implements: final_score = α*relevance + β*recency + γ*importance
    pub async fn retrieve_relevant(
        &self,
        query: &str,
        limit: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        // Get all episodes for scoring (decrypted content for relevance matching)
        let episodes = if project_id.is_some() {
            self.encrypted_storage
                .get_all_episodes_for_scoring_by_project_decrypted(project_id)
                .await?
        } else {
            self.encrypted_storage
                .get_all_episodes_for_scoring_decrypted()
                .await?
        };

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
                let relevance_score = self.calculate_relevance(query, &e.content);
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
        let existing = match self
            .encrypted_storage
            .get_facts_by_project_decrypted(u64::MAX, 0, project_id)
            .await
        {
            Ok(facts) => facts,
            Err(_) => return false,
        };

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
        let mut processed_ids = Vec::new();
        let mut summaries = Vec::new();

        for batch in episodes.chunks(batch_size) {
            // Build a prompt with the batch of memories
            let memories_text: String = batch
                .iter()
                .enumerate()
                .map(|(i, ep)| {
                    let timestamp = &ep.timestamp;
                    let content = if ep.content.len() > 300 {
                        format!("{}...", &ep.content[..300])
                    } else {
                        ep.content.clone()
                    };
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
            match llm.chat(
                "You are a strict memory filter. Only extract genuinely useful, durable personal facts about the user. When in doubt, output NO_FACTS.",
                &consolidation_prompt,
                &[],
                &[],
            ).await {
                Ok(response) => {
                    let text = response.content.trim();
                    if text == "NO_FACTS" || text.is_empty() {
                        // Mark as consolidated even if no facts extracted
                        for ep in batch {
                            processed_ids.push(ep.id.clone());
                        }
                        continue;
                    }

                    // Parse extracted facts
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
                        } else {
                            total_facts += 1;
                        }
                    }

                    for ep in batch {
                        processed_ids.push(ep.id.clone());
                    }
                    summaries.push(format!("Batch: {} episodes -> {} lines", batch.len(), text.lines().count()));
                }
                Err(e) => {
                    tracing::warn!("LLM consolidation batch failed: {}", e);
                    summaries.push(format!("Batch failed: {}", e));
                }
            }
        }

        // Mark processed episodes as consolidated
        if !processed_ids.is_empty() {
            if let Err(e) = self
                .storage
                .mark_episodes_consolidated(&processed_ids)
                .await
            {
                tracing::warn!("Failed to mark episodes consolidated: {}", e);
            }
        }

        let summary = format!(
            "Consolidated {} episodes into {} facts (skipped {} dupes, {} low-quality). {}",
            processed_ids.len(),
            total_facts,
            skipped_dupes,
            skipped_quality,
            summaries.join(" | ")
        );
        tracing::info!("LLM consolidation complete: {}", summary);
        Ok(summary)
    }

    /// Get total entry count
    pub fn entry_count(&self) -> usize {
        self.episode_count.load(Ordering::Relaxed)
    }
}
