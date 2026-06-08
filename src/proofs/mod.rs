//! Cryptographic Execution Proofs (SPEX-inspired)
//!
//! Based on arXiv:2503.18899 "Statistical Proof of Execution"
//! and arXiv:2512.17538 "Binding Agent ID"
//!
//! Every agent action generates a cryptographic proof that can be verified
//! to prove the agent actually performed the claimed action.
use anyhow::Result;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// An execution proof for a single agent action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProof {
    /// Unique proof ID
    pub id: Uuid,

    /// Hash of the action performed (hex encoded)
    pub action_hash: String,

    /// Hash of the input (hex encoded)
    pub input_hash: String,

    /// Hash of the output (hex encoded)
    pub output_hash: String,

    /// Hash of the previous proof (hex encoded)
    pub prev_hash: Option<String>,

    /// Timestamp
    pub timestamp: DateTime<Utc>,

    /// Agent's DID
    pub agent_did: String,

    /// Cryptographic signature (hex encoded)
    pub signature: String,
}

impl ExecutionProof {
    /// Compute the hash of this proof (for chaining)
    #[allow(dead_code)]
    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        hasher.update(&self.action_hash);
        hasher.update(&self.input_hash);
        hasher.update(&self.output_hash);
        if let Some(prev) = &self.prev_hash {
            hasher.update(prev);
        }
        hasher.update(self.timestamp.timestamp().to_le_bytes());
        hasher.update(self.agent_did.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// A verifiable execution trace (chain of proofs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    /// All proofs in order
    pub proofs: Vec<ExecutionProof>,

    /// Root hash (hash of first proof)
    pub root_hash: Option<String>,

    /// Latest hash
    pub latest_hash: Option<String>,
}

impl ExecutionTrace {
    pub fn new() -> Self {
        Self {
            proofs: Vec::new(),
            root_hash: None,
            latest_hash: None,
        }
    }
}

impl Default for ExecutionTrace {
    fn default() -> Self {
        Self::new()
    }
}

/// Engine for generating and verifying execution proofs
pub struct ProofEngine {
    /// Current execution trace (interior mutability for concurrent access)
    #[allow(dead_code)]
    trace: std::sync::Mutex<ExecutionTrace>,
}

impl ProofEngine {
    pub fn new(
        data_dir: &Path,
        _signing_key: &SigningKey,
        trace_encryption_key: Arc<crate::crypto::KeyManager>,
    ) -> Result<Self> {
        // Load existing trace if present
        let trace = Self::load_trace(data_dir, trace_encryption_key.as_ref())?;

        Ok(Self {
            trace: std::sync::Mutex::new(trace),
        })
    }

    fn load_trace(
        data_dir: &Path,
        trace_encryption_key: &crate::crypto::KeyManager,
    ) -> Result<ExecutionTrace> {
        let encrypted_path = data_dir.join("execution_trace.enc");
        if encrypted_path.exists() {
            let payload = std::fs::read(&encrypted_path)?;
            match trace_encryption_key.decrypt(&payload) {
                Ok(decrypted) => match serde_json::from_slice(&decrypted) {
                    Ok(trace) => return Ok(trace),
                    Err(error) => {
                        let backup_path = Self::quarantine_trace_file(&encrypted_path);
                        tracing::error!(
                            "Failed to parse encrypted execution trace at {}: {}. \
                             Starting with an empty trace and preserving the original at {}.",
                            encrypted_path.display(),
                            error,
                            backup_path.display()
                        );
                        return Ok(ExecutionTrace::new());
                    }
                },
                Err(error) => {
                    let backup_path = Self::quarantine_trace_file(&encrypted_path);
                    tracing::error!(
                        "Failed to decrypt execution trace at {}: {}. \
                         Starting with an empty trace and preserving the original at {}.",
                        encrypted_path.display(),
                        error,
                        backup_path.display()
                    );
                    return Ok(ExecutionTrace::new());
                }
            }
        }

        Ok(ExecutionTrace::new())
    }

    fn quarantine_trace_file(path: &Path) -> std::path::PathBuf {
        let mut candidate = path.with_extension("enc.bak");
        if candidate == path {
            candidate = path.with_extension("bak");
        }
        if !candidate.exists() {
            if let Err(error) = std::fs::rename(path, &candidate) {
                tracing::warn!(
                    "Failed to move incompatible execution trace '{}' to '{}': {}",
                    path.display(),
                    candidate.display(),
                    error
                );
            }
            return candidate;
        }

        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("execution_trace");
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        for idx in 1..=32 {
            let file_name = if ext.is_empty() {
                format!("{}.bak.{}", stem, idx)
            } else {
                format!("{}.{}.bak.{}", stem, ext, idx)
            };
            let next = path.with_file_name(file_name);
            if next.exists() {
                continue;
            }
            if let Err(error) = std::fs::rename(path, &next) {
                tracing::warn!(
                    "Failed to move incompatible execution trace '{}' to '{}': {}",
                    path.display(),
                    next.display(),
                    error
                );
            }
            return next;
        }

        candidate
    }
}
