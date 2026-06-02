//! Decentralized Identity (DID) and Verifiable Credentials
//!
//! Implements W3C DID Core specification with did:key method
//! Based on arXiv:2511.02841 "AI Agents with DIDs and VCs"

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Decentralized Identifier (DID) for the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecentralizedId {
    /// The DID string (e.g., "did:key:z6Mk...")
    pub did: String,
    /// The public key (hex encoded)
    pub public_key: String,
}

/// A Verifiable Credential issued to or by the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiableCredential {
    /// Credential ID
    pub id: String,
    /// Credential type
    pub credential_type: Vec<String>,
    /// Who issued this credential
    pub issuer: String,
    /// When it was issued
    pub issuance_date: DateTime<Utc>,
    /// When it expires (optional)
    pub expiration_date: Option<DateTime<Utc>>,
    /// The subject (usually the agent's DID)
    pub subject: CredentialSubject,
    /// Cryptographic proof
    pub proof: CredentialProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialSubject {
    pub id: String,
    pub claims: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialProof {
    pub proof_type: String,
    pub created: DateTime<Utc>,
    pub verification_method: String,
    pub proof_purpose: String,
    pub proof_value: String,
}

/// Manages the agent's identity, keys, and credentials
#[derive(Clone)]
pub struct IdentityManager {
    /// The agent's DID
    did: DecentralizedId,
    /// Signing key (private)
    signing_key: SigningKey,
    /// Verifiable credentials held by this agent
    _credentials: Vec<VerifiableCredential>,
}

impl IdentityManager {
    /// Load existing identity or create a new one
    pub async fn load_or_create(data_dir: &Path) -> Result<Self> {
        let key_path = data_dir.join("identity.key");
        let creds_path = data_dir.join("credentials.json");

        let signing_key = if key_path.exists() {
            // Load existing key
            let key_bytes = std::fs::read(&key_path)?;
            if key_bytes.len() != 32 {
                return Err(anyhow!("Invalid key file"));
            }
            let mut key_array = [0u8; 32];
            key_array.copy_from_slice(&key_bytes);
            SigningKey::from_bytes(&key_array)
        } else {
            // Generate new key
            let mut secret = [0u8; 32];
            rand::rng().fill(&mut secret);
            let signing_key = SigningKey::from_bytes(&secret);
            std::fs::write(&key_path, signing_key.to_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
            }
            signing_key
        };

        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_bytes();

        // Create DID using did:key method
        // Multicodec prefix for Ed25519 public key is 0xed01
        let mut multicodec_key = vec![0xed, 0x01];
        multicodec_key.extend_from_slice(&public_key_bytes);
        let did_string = format!("did:key:z{}", bs58::encode(&multicodec_key).into_string());

        let did = DecentralizedId {
            did: did_string,
            public_key: hex::encode(public_key_bytes),
        };

        // Load credentials if they exist
        let credentials = if creds_path.exists() {
            let content = std::fs::read_to_string(&creds_path)?;
            serde_json::from_str(&content)?
        } else {
            vec![]
        };

        Ok(Self {
            did,
            signing_key,
            _credentials: credentials,
        })
    }

    /// Get the agent's DID string
    pub fn did(&self) -> &str {
        &self.did.did
    }

    /// Get the signing key for creating proofs
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }
}
