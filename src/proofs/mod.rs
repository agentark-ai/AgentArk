//! Cryptographic Execution Proofs (SPEX-inspired)
//!
//! Based on arXiv:2503.18899 "Statistical Proof of Execution"
//! and arXiv:2512.17538 "Binding Agent ID"
//!
//! Every agent action generates a cryptographic proof that can be verified
//! to prove the agent actually performed the claimed action.

use anyhow::Result;
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::ToolCall;

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

    fn data_for_signing(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(self.id.as_bytes());
        data.extend_from_slice(self.action_hash.as_bytes());
        data.extend_from_slice(self.input_hash.as_bytes());
        data.extend_from_slice(self.output_hash.as_bytes());
        if let Some(prev) = &self.prev_hash {
            data.extend_from_slice(prev.as_bytes());
        }
        data.extend_from_slice(&self.timestamp.timestamp().to_le_bytes());
        data.extend_from_slice(self.agent_did.as_bytes());
        data
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
    /// Signing key
    signing_key: SigningKey,

    /// Agent's DID
    agent_did: String,

    /// Current execution trace (interior mutability for concurrent access)
    trace: std::sync::Mutex<ExecutionTrace>,

    /// Storage path
    data_dir: std::path::PathBuf,

    /// Optional encryption key for on-disk trace persistence.
    trace_encryption_key: Option<Arc<crate::crypto::KeyManager>>,
}

impl ProofEngine {
    pub fn new(
        data_dir: &Path,
        signing_key: &SigningKey,
        trace_encryption_key: Option<Arc<crate::crypto::KeyManager>>,
    ) -> Result<Self> {
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_bytes();

        // Reconstruct DID
        let mut multicodec_key = vec![0xed, 0x01];
        multicodec_key.extend_from_slice(&public_key_bytes);
        let agent_did = format!("did:key:z{}", bs58::encode(&multicodec_key).into_string());

        // Load existing trace if present
        let trace = Self::load_trace(data_dir, trace_encryption_key.as_deref())?;

        Ok(Self {
            signing_key: signing_key.clone(),
            agent_did,
            trace: std::sync::Mutex::new(trace),
            data_dir: data_dir.to_path_buf(),
            trace_encryption_key,
        })
    }

    /// Generate a proof for an execution
    pub fn generate_proof(
        &self,
        input: &str,
        output: &str,
        tool_calls: &[ToolCall],
    ) -> Result<ExecutionProof> {
        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        // Hash the action (tool calls)
        let action_hash = Self::hash_data(&serde_json::to_vec(tool_calls)?);

        // Hash input
        let input_hash = Self::hash_data(input.as_bytes());

        // Hash output
        let output_hash = Self::hash_data(output.as_bytes());

        let mut trace = self
            .trace
            .lock()
            .map_err(|e| anyhow::anyhow!("Trace lock poisoned: {}", e))?;

        // Get previous hash for chaining
        let prev_hash = trace.latest_hash.clone();

        // Create unsigned proof
        let mut proof = ExecutionProof {
            id,
            action_hash,
            input_hash,
            output_hash,
            prev_hash,
            timestamp,
            agent_did: self.agent_did.clone(),
            signature: String::new(),
        };

        // Sign the proof
        let data = proof.data_for_signing();
        let signature = self.signing_key.sign(&data);
        proof.signature = hex::encode(signature.to_bytes());

        // Update trace
        let proof_hash = proof.hash();
        if trace.root_hash.is_none() {
            trace.root_hash = Some(proof_hash.clone());
        }
        trace.latest_hash = Some(proof_hash);
        trace.proofs.push(proof.clone());

        drop(trace); // Release lock before I/O
                     // Persist trace
        self.save_trace()?;

        Ok(proof)
    }

    /// Hash arbitrary data
    fn hash_data(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Save trace to disk
    fn save_trace(&self) -> Result<()> {
        let trace = self
            .trace
            .lock()
            .map_err(|e| anyhow::anyhow!("Trace lock poisoned: {}", e))?;
        let content = serde_json::to_string_pretty(&*trace)?;
        if let Some(key_manager) = self.trace_encryption_key.as_ref() {
            let encrypted = key_manager.encrypt(content.as_bytes())?;
            let encrypted_path = self.data_dir.join("execution_trace.enc");
            std::fs::write(encrypted_path, encrypted)?;
            let legacy_path = self.data_dir.join("execution_trace.json");
            if legacy_path.exists() {
                let _ = std::fs::remove_file(legacy_path);
            }
        } else {
            let trace_path = self.data_dir.join("execution_trace.json");
            std::fs::write(trace_path, content)?;
        }
        Ok(())
    }

    fn load_trace(
        data_dir: &Path,
        trace_encryption_key: Option<&crate::crypto::KeyManager>,
    ) -> Result<ExecutionTrace> {
        let encrypted_path = data_dir.join("execution_trace.enc");
        if encrypted_path.exists() {
            let payload = std::fs::read(&encrypted_path)?;
            if let Some(key_manager) = trace_encryption_key {
                let decrypted = key_manager.decrypt(&payload)?;
                return Ok(serde_json::from_slice(&decrypted)?);
            }
            anyhow::bail!("Encrypted proof trace exists but no trace encryption key is available");
        }

        let legacy_path = data_dir.join("execution_trace.json");
        if legacy_path.exists() {
            let content = std::fs::read_to_string(&legacy_path)?;
            return Ok(serde_json::from_str(&content)?);
        }

        Ok(ExecutionTrace::new())
    }

    /// Get a clone of the current execution trace
    pub fn trace(&self) -> ExecutionTrace {
        self.trace.lock().map(|t| t.clone()).unwrap_or_default()
    }
}

/// Compact proof receipt for sharing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofReceipt {
    pub proof_id: Uuid,
    pub action_summary: String,
    pub timestamp: DateTime<Utc>,
    pub agent_did: String,
    pub proof_hash: String,
    pub signature: String,
}

impl From<&ExecutionProof> for ProofReceipt {
    fn from(proof: &ExecutionProof) -> Self {
        Self {
            proof_id: proof.id,
            action_summary: format!("Action hash: {}", hex::encode(&proof.action_hash[..8])),
            timestamp: proof.timestamp,
            agent_did: proof.agent_did.clone(),
            proof_hash: hex::encode(proof.hash()),
            signature: hex::encode(&proof.signature[..32]), // Truncated for display
        }
    }
}
