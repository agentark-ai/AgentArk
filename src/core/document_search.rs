//! Document retrieval helpers for metadata-aware and embedding-aware search.
//!
//! Keep this module free of agent orchestration concerns so document retrieval
//! can evolve without expanding `agent.rs`.

use anyhow::Result;
use sea_orm::entity::prelude::PgVector;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::core::embeddings::EmbeddingClient;
use crate::storage::{document, document_chunk, Storage};

const MAX_EMBED_BATCH: usize = 64;
const MAX_FILENAME_MATCH_DOCS: usize = 4;
const MAX_FILENAME_MATCH_CHUNKS: usize = 3;
const MAX_EXPLICIT_MATCH_CHUNKS: usize = 4;
const MAX_VECTOR_SHORTLIST_CHUNKS: usize = 384;
const MAX_RECENT_SHORTLIST_CHUNKS: usize = 96;
const MIN_LEXICAL_SCORE: f32 = 0.08;
const MIN_DENSE_SCORE: f32 = 0.22;

/// A scored document chunk returned by retrieval helpers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DocumentSearchHit {
    pub document_id: String,
    pub filename: String,
    pub content_type: String,
    pub project_id: Option<String>,
    pub chunk_index: Option<i32>,
    pub created_at: Option<String>,
    pub content: String,
    pub lexical_score: f32,
    pub dense_score: Option<f32>,
    pub score: f32,
    pub match_reason: String,
}

impl DocumentSearchHit {
    pub(crate) fn new(
        document_id: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            project_id: None,
            chunk_index: None,
            created_at: None,
            content: content.into(),
            lexical_score: 0.0,
            dense_score: None,
            score: 0.0,
            match_reason: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct SearchableDocumentMeta {
    document: document::Model,
    normalized_filename: String,
    normalized_stem: String,
    filename_tokens: HashSet<String>,
}

/// Normalize lookup text for filename and chunk matching.
pub(crate) fn normalize_document_lookup_text(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tokenize document search text into a set of useful query tokens.
pub(crate) fn document_search_tokens(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| {
            let trimmed = word.trim();
            if trimmed.len() < 3
                || trimmed.chars().all(|c| c.is_ascii_digit())
                || is_generic_document_query_token(trimmed)
            {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// Tokens that usually indicate a generic document question rather than a
/// filename-specific lookup.
pub(crate) fn is_generic_document_query_token(token: &str) -> bool {
    matches!(
        token,
        "what"
            | "does"
            | "talk"
            | "about"
            | "summarize"
            | "summary"
            | "explain"
            | "review"
            | "document"
            | "documents"
            | "file"
            | "files"
            | "attachment"
            | "attachments"
            | "pdf"
            | "docx"
            | "txt"
            | "md"
            | "csv"
    )
}

/// Build the text fed into the embedding model for a document chunk.
pub(crate) fn build_embedding_text(
    filename: &str,
    content_type: &str,
    chunk_text: &str,
    project_id: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    let filename = filename.trim();
    if !filename.is_empty() {
        parts.push(format!("filename: {}", filename));
    }
    let content_type = content_type.trim();
    if !content_type.is_empty() {
        parts.push(format!("content_type: {}", content_type));
    }
    if let Some(pid) = project_id.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(format!("project_id: {}", pid));
    }
    let content = chunk_text.trim();
    if !content.is_empty() {
        parts.push(format!("content: {}", content));
    }
    parts.join("\n")
}

/// Compute a dot product for equally sized vectors.
pub(crate) fn dot_product(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    Some(a.iter().zip(b.iter()).map(|(x, y)| x * y).sum())
}

/// Compute cosine similarity for arbitrary vectors.
#[cfg(test)]
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    let dot = dot_product(a, b)?;
    let norm_a = a.iter().map(|v| v * v).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        None
    } else {
        Some(dot / (norm_a * norm_b))
    }
}

/// For normalized embeddings, dot product is already the cosine score.
pub(crate) fn normalized_embedding_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    dot_product(a, b)
}

/// Blend lexical and dense scores into a single ranking score.
pub(crate) fn combine_search_scores(
    lexical_score: f32,
    dense_score: Option<f32>,
    filename_boost: f32,
) -> f32 {
    let dense = dense_score.unwrap_or(0.0).clamp(0.0, 1.0);
    (0.55 * lexical_score.clamp(0.0, 1.0) + 0.40 * dense + filename_boost.clamp(0.0, 0.35))
        .clamp(0.0, 1.0)
}

fn document_filename_stem(filename: &str) -> &str {
    filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename)
}

fn build_document_meta(doc: document::Model) -> SearchableDocumentMeta {
    SearchableDocumentMeta {
        normalized_filename: normalize_document_lookup_text(&doc.filename),
        normalized_stem: normalize_document_lookup_text(document_filename_stem(&doc.filename)),
        filename_tokens: document_search_tokens(&doc.filename),
        document: doc,
    }
}

fn filename_match_boost(
    query_normalized: &str,
    filename_query_tokens: &HashSet<String>,
    doc: &SearchableDocumentMeta,
) -> f32 {
    if query_normalized.is_empty() {
        return 0.0;
    }

    let exactish = (!doc.normalized_filename.is_empty()
        && (query_normalized.contains(&doc.normalized_filename)
            || doc.normalized_filename.contains(query_normalized)))
        || (!doc.normalized_stem.is_empty()
            && (query_normalized.contains(&doc.normalized_stem)
                || doc.normalized_stem.contains(query_normalized)));
    if exactish {
        return 0.30;
    }

    if filename_query_tokens.is_empty() || doc.filename_tokens.is_empty() {
        return 0.0;
    }

    let overlap = filename_query_tokens
        .intersection(&doc.filename_tokens)
        .count();
    if overlap == 0 {
        return 0.0;
    }

    (0.10 + 0.18 * overlap as f32 / filename_query_tokens.len() as f32).min(0.28)
}

fn lexical_chunk_score(
    query_normalized: &str,
    query_tokens: &HashSet<String>,
    content: &str,
) -> f32 {
    if query_normalized.is_empty() || query_tokens.is_empty() {
        return 0.0;
    }

    let content_normalized = normalize_document_lookup_text(content);
    let content_tokens = document_search_tokens(content);
    if content_tokens.is_empty() {
        return 0.0;
    }

    let overlap = query_tokens.intersection(&content_tokens).count();
    if overlap == 0 && !content_normalized.contains(query_normalized) {
        return 0.0;
    }

    let coverage = overlap as f32 / query_tokens.len() as f32;
    let phrase_boost = if content_normalized.contains(query_normalized) {
        0.25
    } else {
        0.0
    };
    (coverage + phrase_boost).min(1.0)
}

fn dense_chunk_score(query_embedding: &PgVector, embedding: Option<&PgVector>) -> Option<f32> {
    normalized_embedding_similarity(query_embedding.as_slice(), embedding?.as_slice())
        .map(|score| score.clamp(0.0, 1.0))
}

fn hit_key(hit: &DocumentSearchHit) -> String {
    match hit.chunk_index {
        Some(chunk_index) => format!("{}::{}", hit.document_id, chunk_index),
        None => format!("{}::{}", hit.document_id, hit.content),
    }
}

fn merge_match_reasons(existing: &str, new_reason: &str) -> String {
    let mut merged = Vec::new();
    for reason in [existing, new_reason]
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !merged
            .iter()
            .any(|existing_reason: &String| existing_reason == reason)
        {
            merged.push(reason.to_string());
        }
    }
    merged.join(", ")
}

fn merge_hit(hits: &mut HashMap<String, DocumentSearchHit>, hit: DocumentSearchHit) {
    let key = hit_key(&hit);
    match hits.get_mut(&key) {
        Some(existing) => {
            existing.score = existing.score.max(hit.score);
            existing.lexical_score = existing.lexical_score.max(hit.lexical_score);
            existing.dense_score = match (existing.dense_score, hit.dense_score) {
                (Some(lhs), Some(rhs)) => Some(lhs.max(rhs)),
                (None, Some(rhs)) => Some(rhs),
                (Some(lhs), None) => Some(lhs),
                (None, None) => None,
            };
            existing.match_reason = merge_match_reasons(&existing.match_reason, &hit.match_reason);
        }
        None => {
            hits.insert(key, hit);
        }
    }
}

fn sort_and_truncate_hits(
    mut hits: Vec<DocumentSearchHit>,
    limit: usize,
) -> Vec<DocumentSearchHit> {
    hits.sort_by(|lhs, rhs| {
        rhs.score
            .partial_cmp(&lhs.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                rhs.dense_score
                    .unwrap_or(0.0)
                    .partial_cmp(&lhs.dense_score.unwrap_or(0.0))
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| {
                rhs.lexical_score
                    .partial_cmp(&lhs.lexical_score)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| lhs.filename.cmp(&rhs.filename))
            .then_with(|| {
                lhs.chunk_index
                    .unwrap_or_default()
                    .cmp(&rhs.chunk_index.unwrap_or_default())
            })
    });
    hits.truncate(limit);
    hits
}

fn build_chunk_hit(
    doc: &SearchableDocumentMeta,
    chunk: &document_chunk::Model,
) -> DocumentSearchHit {
    let mut hit = DocumentSearchHit::new(
        chunk.document_id.clone(),
        doc.document.filename.clone(),
        doc.document.content_type.clone(),
        chunk.content.clone(),
    );
    hit.project_id = doc.document.project_id.clone();
    hit.chunk_index = Some(chunk.chunk_index);
    hit.created_at = Some(doc.document.created_at.clone());
    hit
}

fn build_match_reason(
    explicit_ref: bool,
    filename_boost: f32,
    lexical_score: f32,
    dense_score: Option<f32>,
) -> String {
    let mut reasons = Vec::new();
    if explicit_ref {
        reasons.push("explicit_doc_ref");
    }
    if filename_boost >= 0.28 {
        reasons.push("filename_exact");
    } else if filename_boost > 0.0 {
        reasons.push("filename");
    }
    if lexical_score >= MIN_LEXICAL_SCORE {
        reasons.push("lexical");
    }
    if dense_score.unwrap_or(0.0) >= MIN_DENSE_SCORE {
        reasons.push("dense");
    }
    if reasons.is_empty() {
        reasons.push("document");
    }
    reasons.join(", ")
}

async fn embed_chunks_with_metadata(
    embedding_client: &EmbeddingClient,
    filename: &str,
    content_type: &str,
    project_id: Option<&str>,
    chunks: &mut [document_chunk::Model],
    chunk_indices: &[usize],
) -> Result<usize> {
    let mut updated = 0usize;
    for batch in chunk_indices.chunks(MAX_EMBED_BATCH) {
        let texts: Vec<String> = batch
            .iter()
            .map(|index| {
                build_embedding_text(filename, content_type, &chunks[*index].content, project_id)
            })
            .collect();
        let embeddings = embedding_client.embed_texts(&texts).await?;
        if embeddings.len() != batch.len() {
            tracing::warn!(
                "Document embedding batch mismatch: expected {}, got {}",
                batch.len(),
                embeddings.len()
            );
            continue;
        }

        for (offset, embedding) in embeddings.into_iter().enumerate() {
            let chunk_index = batch[offset];
            chunks[chunk_index].embedding = Some(embedding);
            updated += 1;
        }
    }
    Ok(updated)
}

/// Generate embeddings for newly created document chunks before they are stored.
pub(crate) async fn embed_document_chunks(
    embedding_client: Option<&EmbeddingClient>,
    filename: &str,
    content_type: &str,
    project_id: Option<&str>,
    chunks: &mut [document_chunk::Model],
) -> Result<usize> {
    let Some(embedding_client) = embedding_client else {
        return Ok(0);
    };
    let candidate_indices: Vec<usize> = chunks
        .iter()
        .enumerate()
        .filter_map(|(index, chunk)| (!chunk.content.trim().is_empty()).then_some(index))
        .collect();
    if candidate_indices.is_empty() {
        return Ok(0);
    }
    embed_chunks_with_metadata(
        embedding_client,
        filename,
        content_type,
        project_id,
        chunks,
        &candidate_indices,
    )
    .await
}

fn dense_chunk_shortlist_limit(limit: usize) -> u64 {
    limit
        .max(1)
        .saturating_mul(12)
        .clamp(96, MAX_VECTOR_SHORTLIST_CHUNKS) as u64
}

fn recent_chunk_shortlist_limit(limit: usize) -> u64 {
    limit
        .max(1)
        .saturating_mul(4)
        .clamp(24, MAX_RECENT_SHORTLIST_CHUNKS) as u64
}

fn merge_shortlist_ids(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(primary.len() + secondary.len());
    for id in primary.into_iter().chain(secondary) {
        if seen.insert(id.clone()) {
            merged.push(id);
        }
    }
    merged
}

/// Search indexed document chunks using explicit refs, filename metadata,
/// lexical overlap, and dense similarity from the local embedding model.
pub(crate) async fn search_documents(
    storage: &Storage,
    embedding_client: Option<&EmbeddingClient>,
    query: &str,
    limit: usize,
    project_id: Option<&str>,
) -> Result<Vec<DocumentSearchHit>> {
    let docs = storage.list_documents_for_search(project_id).await?;
    search_document_models(storage, embedding_client, query, limit, docs).await
}

/// Search a caller-supplied document set. This keeps product/runtime help
/// retrieval scoped to its own indexed corpus instead of searching every
/// uploaded document.
pub(crate) async fn search_document_models(
    storage: &Storage,
    embedding_client: Option<&EmbeddingClient>,
    query: &str,
    limit: usize,
    docs: Vec<document::Model>,
) -> Result<Vec<DocumentSearchHit>> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc_ref_re = regex::Regex::new(r"(?i)\bdoc:([a-z0-9-]{6,})\b").ok();
    let explicit_doc_ids: HashSet<String> = doc_ref_re
        .as_ref()
        .map(|re| {
            re.captures_iter(query)
                .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
                .collect()
        })
        .unwrap_or_default();

    let query_without_refs = if let Some(re) = doc_ref_re.as_ref() {
        re.replace_all(query, " ").to_string()
    } else {
        query.to_string()
    };
    let query_normalized = normalize_document_lookup_text(&query_without_refs);
    let query_tokens = document_search_tokens(&query_without_refs);
    let filename_query_tokens: HashSet<String> = query_tokens
        .iter()
        .filter(|token| !is_generic_document_query_token(token))
        .cloned()
        .collect();

    if docs.is_empty() {
        return Ok(Vec::new());
    }

    let documents_by_id: HashMap<String, SearchableDocumentMeta> = docs
        .into_iter()
        .map(build_document_meta)
        .map(|doc| (doc.document.id.clone(), doc))
        .collect();

    let filename_boosts: HashMap<String, f32> = documents_by_id
        .iter()
        .filter_map(|(doc_id, doc)| {
            let boost = filename_match_boost(&query_normalized, &filename_query_tokens, doc);
            (boost > 0.0).then_some((doc_id.clone(), boost))
        })
        .collect();

    let mut hits = HashMap::new();

    for doc_id in &explicit_doc_ids {
        let Some(doc) = documents_by_id.get(doc_id) else {
            continue;
        };
        match storage.get_document_chunks(doc_id).await {
            Ok(doc_chunks) => {
                for (position, chunk) in doc_chunks
                    .into_iter()
                    .take(MAX_EXPLICIT_MATCH_CHUNKS)
                    .enumerate()
                {
                    let mut hit = build_chunk_hit(doc, &chunk);
                    hit.score = (0.98 - position as f32 * 0.02).clamp(0.0, 1.0);
                    hit.match_reason = "explicit_doc_ref".to_string();
                    merge_hit(&mut hits, hit);
                }
            }
            Err(error) => {
                tracing::warn!(
                    doc_id = doc_id,
                    "Explicit document chunk load failed: {}",
                    error
                );
            }
        }
    }

    let mut filename_docs: Vec<(&SearchableDocumentMeta, f32)> = filename_boosts
        .iter()
        .filter_map(|(doc_id, boost)| documents_by_id.get(doc_id).map(|doc| (doc, *boost)))
        .collect();
    filename_docs.sort_by(|lhs, rhs| rhs.1.partial_cmp(&lhs.1).unwrap_or(Ordering::Equal));

    for (doc, boost) in filename_docs
        .into_iter()
        .take(MAX_FILENAME_MATCH_DOCS.max(limit))
    {
        match storage.get_document_chunks(&doc.document.id).await {
            Ok(doc_chunks) => {
                for (position, chunk) in doc_chunks
                    .into_iter()
                    .take(MAX_FILENAME_MATCH_CHUNKS)
                    .enumerate()
                {
                    let mut hit = build_chunk_hit(doc, &chunk);
                    hit.score = (0.58 + boost - position as f32 * 0.03).clamp(0.0, 1.0);
                    hit.match_reason = if boost >= 0.28 {
                        "filename_exact".to_string()
                    } else {
                        "filename".to_string()
                    };
                    merge_hit(&mut hits, hit);
                }
            }
            Err(error) => {
                tracing::warn!(
                    doc_id = doc.document.id.as_str(),
                    "Filename-boosted document chunk load failed: {}",
                    error
                );
            }
        }
    }

    if query_tokens.is_empty() {
        return Ok(sort_and_truncate_hits(hits.into_values().collect(), limit));
    }

    let query_embedding = if let Some(client) = embedding_client {
        match client
            .embed_texts(std::slice::from_ref(&query_without_refs))
            .await
        {
            Ok(mut embeddings) => embeddings.drain(..).next(),
            Err(error) => {
                tracing::warn!(
                    "Document query embedding failed; falling back to lexical search: {}",
                    error
                );
                None
            }
        }
    } else {
        None
    };
    let doc_ids: Vec<String> = documents_by_id.keys().cloned().collect();
    let chunks = if let Some(embedding) = query_embedding.as_ref() {
        let dense_ids = match storage
            .nearest_document_chunk_ids(embedding, &doc_ids, dense_chunk_shortlist_limit(limit))
            .await
        {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!("Document pgvector shortlist failed: {}", error);
                Vec::new()
            }
        };
        let recent_ids = match storage
            .list_recent_document_chunk_ids(&doc_ids, recent_chunk_shortlist_limit(limit))
            .await
        {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!("Document recent shortlist failed: {}", error);
                Vec::new()
            }
        };
        let shortlist_ids = merge_shortlist_ids(dense_ids, recent_ids);
        if shortlist_ids.is_empty() {
            tracing::debug!(
                documents = doc_ids.len(),
                "Document shortlist empty; falling back to full document chunk scan"
            );
            storage.list_document_chunks_for_documents(&doc_ids).await?
        } else {
            match storage.get_document_chunks_by_ids(&shortlist_ids).await {
                Ok(chunks) => chunks,
                Err(error) => {
                    tracing::warn!(
                        ids = shortlist_ids.len(),
                        "Document shortlist hydrate failed; falling back to full scan: {}",
                        error
                    );
                    storage.list_document_chunks_for_documents(&doc_ids).await?
                }
            }
        }
    } else {
        storage.list_document_chunks_for_documents(&doc_ids).await?
    };
    if chunks.is_empty() {
        return Ok(sort_and_truncate_hits(hits.into_values().collect(), limit));
    }

    for chunk in chunks {
        let Some(doc) = documents_by_id.get(&chunk.document_id) else {
            continue;
        };

        let filename_boost = filename_boosts
            .get(&chunk.document_id)
            .copied()
            .unwrap_or(0.0);
        let lexical_score = lexical_chunk_score(&query_normalized, &query_tokens, &chunk.content);
        let dense_score = query_embedding
            .as_ref()
            .and_then(|embedding| dense_chunk_score(embedding, chunk.embedding.as_ref()));

        if lexical_score < MIN_LEXICAL_SCORE && dense_score.unwrap_or(0.0) < MIN_DENSE_SCORE {
            continue;
        }

        let mut hit = build_chunk_hit(doc, &chunk);
        hit.lexical_score = lexical_score;
        hit.dense_score = dense_score;
        hit.score = combine_search_scores(lexical_score, dense_score, filename_boost);
        hit.match_reason = build_match_reason(
            explicit_doc_ids.contains(&chunk.document_id),
            filename_boost,
            lexical_score,
            dense_score,
        );
        merge_hit(&mut hits, hit);
    }

    Ok(sort_and_truncate_hits(hits.into_values().collect(), limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_normalization_splits_punctuation() {
        let normalized =
            normalize_document_lookup_text("NEW_TENANCY_AGREEMENT_2026_under_200kb.pdf");
        assert_eq!(normalized, "new tenancy agreement 2026 under 200kb pdf");
    }

    #[test]
    fn document_tokens_drop_noise_but_keep_filename_terms() {
        let tokens = document_search_tokens(
            "what does NEW_TENANCY_AGREEMENT_2026_under_200kb.pdf talk about?",
        );
        assert!(tokens.contains("new"));
        assert!(tokens.contains("tenancy"));
        assert!(tokens.contains("agreement"));
        assert!(tokens.contains("200kb"));
        assert!(!tokens.contains("what"));
        assert!(!tokens.contains("does"));
    }

    #[test]
    fn embedding_text_includes_metadata() {
        let text = build_embedding_text(
            "lease.pdf",
            "application/pdf",
            "rent is due on the first",
            Some("project-123"),
        );
        assert!(text.contains("filename: lease.pdf"));
        assert!(text.contains("content_type: application/pdf"));
        assert!(text.contains("project_id: project-123"));
        assert!(text.contains("content: rent is due on the first"));
    }

    #[test]
    fn similarity_matches_for_normalized_vectors() {
        let a = vec![0.6, 0.8];
        let b = vec![0.6, 0.8];
        assert_eq!(dot_product(&a, &b), Some(1.0));
        assert_eq!(cosine_similarity(&a, &b), Some(1.0));
        assert_eq!(normalized_embedding_similarity(&a, &b), Some(1.0));
    }

    #[test]
    fn combine_scores_weights_dense_and_lexical() {
        let score = combine_search_scores(0.8, Some(0.5), 0.1);
        assert!(score > 0.0);
        assert!(score <= 1.0);
    }
}
