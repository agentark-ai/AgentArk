//! Encryption module for securing stored data
//!
//! Uses AES-256-GCM for encryption and Argon2 for key derivation.
//! All sensitive data (API keys, tokens, memories) are encrypted at rest.

pub mod master;

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use argon2::{Argon2, Params};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Encrypted data format: salt (16 bytes) + nonce (12 bytes) + ciphertext
pub(crate) const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
pub(crate) const KEY_LEN: usize = 32; // AES-256

/// Master encryption key manager
///
/// The key is derived from a master password or loaded from a secure keyfile.
/// For local-only deployment, we use a machine-specific key derived from
/// hardware identifiers if no password is set.
pub struct KeyManager {
    /// The derived encryption key (zeroed on drop)
    key: Zeroizing<[u8; KEY_LEN]>,
}

impl KeyManager {
    /// Create a new KeyManager from a password
    pub fn from_password(password: &str, salt: &[u8]) -> Result<Self> {
        let key = derive_key(password.as_bytes(), salt)?;
        Ok(Self {
            key: Zeroizing::new(key),
        })
    }

    /// Load or create the master key from a keyfile
    /// If the keyfile doesn't exist, generates a new random key
    pub fn load_or_create(keyfile_path: &Path) -> Result<Self> {
        if keyfile_path.exists() {
            return Self::load_existing_keyfile(keyfile_path);
        }

        // Generate a new random key and publish it atomically so concurrent
        // split-service startups cannot observe a partially written keyfile or
        // overwrite each other with different bootstrap material.
        let mut generated = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut generated);

        match atomic_write_file_if_absent(keyfile_path, &generated)? {
            true => Ok(Self {
                key: Zeroizing::new(generated),
            }),
            false => Self::load_existing_keyfile(keyfile_path),
        }
    }

    fn load_existing_keyfile(keyfile_path: &Path) -> Result<Self> {
        for attempt in 0..20 {
            match fs::read(keyfile_path) {
                Ok(key_data) if key_data.len() == KEY_LEN => {
                    let mut key = [0u8; KEY_LEN];
                    key.copy_from_slice(&key_data);
                    return Ok(Self {
                        key: Zeroizing::new(key),
                    });
                }
                Ok(key_data) if attempt < 19 && key_data.len() < KEY_LEN => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(key_data) => {
                    return Err(anyhow!(
                        "Invalid keyfile length: expected {} bytes, got {} bytes at {:?}",
                        KEY_LEN,
                        key_data.len(),
                        keyfile_path
                    ));
                }
                Err(error) if attempt < 19 && error.kind() == std::io::ErrorKind::NotFound => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(anyhow!(
                        "Failed to read keyfile at {:?}: {}",
                        keyfile_path,
                        error
                    ));
                }
            }
        }
        unreachable!("keyfile read loop should have returned or errored");
    }

    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"agentark-storage-key-fingerprint-v1");
        hasher.update(&self.key[..]);
        hex::encode(hasher.finalize())
    }

    /// Encrypt data using the master key
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&*self.key)
            .map_err(|e| anyhow!("Failed to create cipher: {}", e))?;

        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        // Combine nonce + ciphertext
        let mut result = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt data using the master key
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<Vec<u8>> {
        if encrypted.len() < NONCE_LEN {
            return Err(anyhow!("Invalid encrypted data: too short"));
        }

        let cipher = Aes256Gcm::new_from_slice(&*self.key)
            .map_err(|e| anyhow!("Failed to create cipher: {}", e))?;

        let nonce = Nonce::from_slice(&encrypted[..NONCE_LEN]);
        let ciphertext = &encrypted[NONCE_LEN..];

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption failed: {}", e))
    }

    /// Encrypt a string, returning base64-encoded result
    pub fn encrypt_string(&self, plaintext: &str) -> Result<String> {
        let encrypted = self.encrypt(plaintext.as_bytes())?;
        Ok(BASE64.encode(&encrypted))
    }

    /// Decrypt a base64-encoded string
    pub fn decrypt_string(&self, encrypted_b64: &str) -> Result<String> {
        let encrypted = BASE64
            .decode(encrypted_b64)
            .map_err(|e| anyhow!("Invalid base64: {}", e))?;
        let decrypted = self.decrypt(&encrypted)?;
        String::from_utf8(decrypted).map_err(|e| anyhow!("Invalid UTF-8: {}", e))
    }
}

/// Derive an encryption key from a password using Argon2id
pub(crate) fn derive_key(password: &[u8], salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    // Use Argon2id with secure parameters
    let params = Params::new(
        65536, // 64 MiB memory
        3,     // 3 iterations
        4,     // 4 parallel lanes
        Some(KEY_LEN),
    )
    .map_err(|e| anyhow!("Invalid Argon2 params: {}", e))?;

    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| anyhow!("Key derivation failed: {}", e))?;

    Ok(key)
}

/// Generate a random salt for key derivation
pub(crate) fn generate_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(windows)]
fn move_file_replace_windows(source: &Path, destination: &Path) -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let source_wide = wide(source.as_os_str());
    let destination_wide = wide(destination.as_os_str());
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if moved == 0 {
        return Err(anyhow!(
            "Failed to atomically replace {:?}: {}",
            destination,
            std::io::Error::last_os_error()
        ));
    }

    Ok(())
}

pub(crate) fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Cannot determine parent directory for {:?}", path))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file_mut().sync_all()?;
    let temp_path = temp.into_temp_path();

    #[cfg(windows)]
    {
        let source_path: PathBuf = temp_path.to_path_buf();
        move_file_replace_windows(&source_path, path)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        temp_path
            .persist(path)
            .map_err(|err| anyhow!("Failed to atomically replace {:?}: {}", path, err.error))?;
        Ok(())
    }
}

pub(crate) fn atomic_write_file_if_absent(path: &Path, contents: &[u8]) -> Result<bool> {
    let _parent = path
        .parent()
        .ok_or_else(|| anyhow!("Cannot determine parent directory for {:?}", path))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(contents)?;
            file.sync_all()?;
            Ok(true)
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                || (path.exists() && error.kind() == std::io::ErrorKind::PermissionDenied) =>
        {
            Ok(false)
        }
        Err(error) => Err(anyhow!("Failed to atomically create {:?}: {}", path, error)),
    }
}

/// Generate a self-signed TLS certificate for localhost
/// Returns (cert_pem, key_pem) as strings
#[cfg(feature = "tls")]
pub fn generate_self_signed_cert(data_dir: &Path) -> Result<(String, String)> {
    let cert_path = data_dir.join("tls_cert.pem");
    let key_path = data_dir.join("tls_key.pem");

    // Reuse existing cert if available
    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        return Ok((cert_pem, key_pem));
    }

    // Generate new self-signed certificate
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName("localhost".try_into()?),
        rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
    ];

    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    // Save for reuse
    std::fs::write(&cert_path, &cert_pem)?;
    std::fs::write(&key_path, &key_pem)?;

    tracing::info!("Generated self-signed TLS certificate at {:?}", cert_path);

    Ok((cert_pem, key_pem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let salt = generate_salt();
        let km = KeyManager::from_password("test-password", &salt).unwrap();

        let plaintext = "Hello, World! This is a secret message.";
        let encrypted = km.encrypt(plaintext.as_bytes()).unwrap();
        let decrypted = km.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext.as_bytes(), &decrypted[..]);
    }

    #[test]
    fn test_encrypt_decrypt_string() {
        let salt = generate_salt();
        let km = KeyManager::from_password("test-password", &salt).unwrap();

        let plaintext = "API_KEY_12345";
        let encrypted = km.encrypt_string(plaintext).unwrap();
        let decrypted = km.decrypt_string(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_key_derivation() {
        let password = b"my_secure_password";
        let salt = generate_salt();

        let key1 = derive_key(password, &salt).unwrap();
        let key2 = derive_key(password, &salt).unwrap();

        // Same password + salt = same key
        assert_eq!(key1, key2);

        // Different salt = different key
        let salt2 = generate_salt();
        let key3 = derive_key(password, &salt2).unwrap();
        assert_ne!(key1, key3);
    }
}
