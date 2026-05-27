//! Master password management for AgentArk
//!
//! When a master password is set, all encryption keys are derived from it
//! via Argon2id. The password itself is never stored - only a salt and a
//! verification hash (derived with a separate salt so it cannot reveal
//! the encryption key).
//!
//! Persisted file: `config_dir/master.json`

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{derive_key, generate_salt, KeyManager, KEY_LEN};

pub const INSTALL_MASTER_SECRET_PATH: &str = "/run/secrets/agentark_master_key";

/// Persisted master password metadata
#[derive(serde::Serialize, serde::Deserialize)]
struct MasterMeta {
    /// Salt for encryption key derivation (hex-encoded)
    salt: String,
    /// Salt for verification hash (hex-encoded, different from encryption salt)
    verification_salt: String,
    /// Argon2id hash output for password verification (hex-encoded)
    verification_hash: String,
    /// Schema version for future upgrades
    version: u32,
    /// True when this installation is using a generated bootstrap password.
    #[serde(default)]
    bootstrap: bool,
    /// True when this installation is using the Docker install-managed secret.
    #[serde(default)]
    install_managed: bool,
}

pub struct PreparedMasterPassword {
    pub(crate) key_manager: Arc<KeyManager>,
    meta_json: String,
    bootstrap: bool,
    install_managed: bool,
}

pub struct MasterPasswordManager {
    config_dir: PathBuf,
    _data_dir: PathBuf,
}

impl MasterPasswordManager {
    pub fn new(config_dir: &Path, data_dir: &Path) -> Self {
        Self {
            config_dir: config_dir.to_path_buf(),
            _data_dir: data_dir.to_path_buf(),
        }
    }

    fn meta_path(&self) -> PathBuf {
        self.config_dir.join("master.json")
    }

    fn keyfile_path(&self) -> PathBuf {
        self.config_dir.join(".keyfile")
    }

    pub fn read_install_master_secret() -> Result<Option<String>> {
        let path = Path::new(INSTALL_MASTER_SECRET_PATH);
        match std::fs::read_to_string(path) {
            Ok(value) => {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    return Err(anyhow!(
                        "Install-managed encryption secret at {} is empty",
                        INSTALL_MASTER_SECRET_PATH
                    ));
                }
                Ok(Some(trimmed))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(anyhow!(
                "Failed to read install-managed encryption secret at {}: {}",
                INSTALL_MASTER_SECRET_PATH,
                error
            )),
        }
    }

    pub fn docker_stack_requires_install_master_secret() -> bool {
        std::env::var("AGENTARK_STACK_ROLE")
            .ok()
            .map(|role| role.trim().to_ascii_lowercase())
            .is_some_and(|role| matches!(role.as_str(), "control" | "executor"))
    }

    fn derive_bootstrap_password(&self) -> Result<String> {
        let keyfile = self.keyfile_path();
        // Ensure keyfile exists.
        let _ = KeyManager::load_or_create(&keyfile)?;
        let key_data = std::fs::read(&keyfile)
            .map_err(|e| anyhow!("Failed to read bootstrap keyfile at {:?}: {}", keyfile, e))?;
        if key_data.len() != KEY_LEN {
            return Err(anyhow!(
                "Invalid bootstrap keyfile length at {:?}: expected {} bytes, got {}",
                keyfile,
                KEY_LEN,
                key_data.len()
            ));
        }

        let mut material = Vec::with_capacity(32 + key_data.len());
        material.extend_from_slice(b"agentark-bootstrap-v1:");
        material.extend_from_slice(&key_data);
        Ok(format!("ak_bootstrap_{}", URL_SAFE_NO_PAD.encode(material)))
    }

    /// Check whether a master password has been configured
    pub fn is_password_set(&self) -> bool {
        self.meta_path().exists()
    }

    pub fn is_bootstrap_password_active(&self) -> Result<bool> {
        if !self.is_password_set() {
            return Ok(false);
        }
        Ok(self.load_meta()?.bootstrap)
    }

    pub fn is_install_managed_password_active(&self) -> Result<bool> {
        if !self.is_password_set() {
            return Ok(false);
        }
        Ok(self.load_meta()?.install_managed)
    }

    pub fn bootstrap_password_if_active(&self) -> Result<Option<String>> {
        if self.is_bootstrap_password_active()? {
            Ok(Some(self.derive_bootstrap_password()?))
        } else {
            Ok(None)
        }
    }

    /// Verify password and return the derived encryption key
    pub fn unlock(&self, password: &str) -> Result<Arc<KeyManager>> {
        let meta = self.load_meta()?;

        // Verify password against stored hash
        let v_salt = hex::decode(&meta.verification_salt)
            .map_err(|_| anyhow!("Corrupt master.json: bad verification_salt hex"))?;
        let v_hash = derive_key(password.as_bytes(), &v_salt)?;
        let expected = hex::decode(&meta.verification_hash)
            .map_err(|_| anyhow!("Corrupt master.json: bad verification_hash hex"))?;

        if v_hash[..] != expected[..] {
            return Err(anyhow!("Invalid master password"));
        }

        // Derive the encryption key (using the encryption salt, not the verification salt)
        let enc_salt =
            hex::decode(&meta.salt).map_err(|_| anyhow!("Corrupt master.json: bad salt hex"))?;
        let km = KeyManager::from_password(password, &enc_salt)?;
        Ok(Arc::new(km))
    }

    pub fn prepare_password(&self, password: &str) -> Result<PreparedMasterPassword> {
        self.prepare_password_with_mode(password, false, false)
    }

    pub fn prepare_install_managed_password(
        &self,
        password: &str,
    ) -> Result<PreparedMasterPassword> {
        self.prepare_password_with_mode(password, false, true)
    }

    fn prepare_password_with_mode(
        &self,
        password: &str,
        bootstrap: bool,
        install_managed: bool,
    ) -> Result<PreparedMasterPassword> {
        // Generate separate salts for encryption and verification
        let enc_salt = generate_salt();
        let v_salt = generate_salt();

        // Derive the encryption key
        let km = KeyManager::from_password(password, &enc_salt)?;

        // Derive the verification hash (separate derivation, separate salt)
        let v_hash = derive_key(password.as_bytes(), &v_salt)?;

        // Write metadata
        let meta = MasterMeta {
            salt: hex::encode(enc_salt),
            verification_salt: hex::encode(v_salt),
            verification_hash: hex::encode(v_hash),
            version: 1,
            bootstrap,
            install_managed,
        };
        let json = serde_json::to_string_pretty(&meta)?;
        Ok(PreparedMasterPassword {
            key_manager: Arc::new(km),
            meta_json: json,
            bootstrap,
            install_managed,
        })
    }

    pub fn commit_prepared_password(&self, prepared: PreparedMasterPassword) -> Result<()> {
        crate::crypto::atomic_write_file(&self.meta_path(), prepared.meta_json.as_bytes())?;

        if prepared.bootstrap {
            tracing::info!(
                "Bootstrap master password initialized (per-install, derived from local keyfile)"
            );
        } else if prepared.install_managed {
            tracing::info!("Master password initialized from install-managed Docker secret volume");
        } else {
            tracing::info!("Master password set - encryption keys derived from password");
        }
        Ok(())
    }

    /// Initialize from the install-managed secret without racing split services.
    /// If another service wins the metadata write, this unlocks with the same
    /// secret and the committed salts.
    pub fn initialize_startup_password_if_needed(&self, password: &str) -> Result<Arc<KeyManager>> {
        if self.is_password_set() {
            return self.unlock(password);
        }

        let prepared = self.prepare_install_managed_password(password)?;
        let key = prepared.key_manager.clone();
        if crate::crypto::atomic_write_file_if_absent(
            &self.meta_path(),
            prepared.meta_json.as_bytes(),
        )? {
            tracing::info!("Master password initialized from install-managed startup secret");
            return Ok(key);
        }

        self.unlock(password)
    }

    /// Initialize a per-install bootstrap password if no master password is configured.
    /// Returns `Some(key)` only when bootstrap initialization occurred.
    pub fn initialize_bootstrap_password_if_needed(&self) -> Result<Option<Arc<KeyManager>>> {
        if self.is_password_set() {
            return Ok(None);
        }
        let bootstrap_password = self.derive_bootstrap_password()?;
        let prepared = self.prepare_password_with_mode(&bootstrap_password, true, false)?;
        let key = prepared.key_manager.clone();

        if crate::crypto::atomic_write_file_if_absent(
            &self.meta_path(),
            prepared.meta_json.as_bytes(),
        )? {
            tracing::info!(
                "Bootstrap master password initialized (per-install, derived from local keyfile)"
            );
            return Ok(Some(key));
        }

        // Another service already initialized the shared config volume. Reuse the
        // persisted bootstrap password instead of keeping a divergent in-memory key.
        if self.is_bootstrap_password_active()? {
            return Ok(Some(self.unlock(&bootstrap_password)?));
        }

        Ok(None)
    }

    /// Remove master password - revert to auto-generated keyfile
    /// Caller is responsible for re-encrypting data with the returned key
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn remove_password(&self) -> Result<Arc<KeyManager>> {
        let km = self.prepare_keyfile_encryption()?;
        self.commit_password_removal()?;
        Ok(km)
    }

    pub fn prepare_keyfile_encryption(&self) -> Result<Arc<KeyManager>> {
        let keyfile = self.config_dir.join(".keyfile");
        Ok(Arc::new(KeyManager::load_or_create(&keyfile)?))
    }

    pub fn commit_password_removal(&self) -> Result<()> {
        let meta_path = self.meta_path();
        if meta_path.exists() {
            std::fs::remove_file(&meta_path)?;
        }

        tracing::info!("Master password removed - reverted to keyfile encryption");
        Ok(())
    }

    fn load_meta(&self) -> Result<MasterMeta> {
        let path = self.meta_path();
        if !path.exists() {
            return Err(anyhow!(
                "No master password configured (master.json not found)"
            ));
        }
        for attempt in 0..20 {
            match std::fs::read_to_string(&path) {
                Ok(content) if !content.trim().is_empty() => match serde_json::from_str(&content) {
                    Ok(meta) => return Ok(meta),
                    Err(error) if attempt < 19 => {
                        let _ = error;
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.into()),
                },
                Ok(_) if attempt < 19 => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(_) => return Err(anyhow!("Corrupt master.json: empty file")),
                Err(error) if attempt < 19 && error.kind() == std::io::ErrorKind::NotFound => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!("master metadata read loop should have returned or errored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_unlock() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = MasterPasswordManager::new(tmp.path(), tmp.path());

        assert!(!mgr.is_password_set());

        let prepared = mgr.prepare_password("test-password-123").unwrap();
        let key = prepared.key_manager.clone();
        mgr.commit_prepared_password(prepared).unwrap();
        assert!(mgr.is_password_set());
        assert!(!mgr.is_bootstrap_password_active().unwrap());

        // Correct password unlocks
        let key2 = mgr.unlock("test-password-123").unwrap();

        // Both keys should produce same encryption results
        let plaintext = b"hello world";
        let encrypted = key.encrypt(plaintext).unwrap();
        let decrypted = key2.decrypt(&encrypted).unwrap();
        assert_eq!(plaintext, &decrypted[..]);

        // Wrong password fails
        assert!(mgr.unlock("wrong-password").is_err());
    }

    #[test]
    fn test_bootstrap_password_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = MasterPasswordManager::new(tmp.path(), tmp.path());

        let key = mgr
            .initialize_bootstrap_password_if_needed()
            .unwrap()
            .expect("bootstrap should initialize on first run");

        assert!(mgr.is_password_set());
        assert!(mgr.is_bootstrap_password_active().unwrap());

        let bootstrap = mgr
            .bootstrap_password_if_active()
            .unwrap()
            .expect("bootstrap password should exist");

        let unlocked = mgr.unlock(&bootstrap).unwrap();
        let plaintext = b"bootstrap roundtrip";
        let encrypted = key.encrypt(plaintext).unwrap();
        let decrypted = unlocked.decrypt(&encrypted).unwrap();
        assert_eq!(plaintext, &decrypted[..]);
    }

    #[test]
    fn bootstrap_initialization_is_atomic_across_concurrent_callers() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = std::sync::Arc::new(MasterPasswordManager::new(tmp.path(), tmp.path()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));

        let handles = (0..8)
            .map(|_| {
                let mgr = mgr.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    mgr.initialize_bootstrap_password_if_needed()
                        .expect("bootstrap init should succeed")
                        .expect("bootstrap init should yield a key")
                })
            })
            .collect::<Vec<_>>();

        let keys = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread should complete"))
            .collect::<Vec<_>>();

        let bootstrap_password = mgr
            .bootstrap_password_if_active()
            .expect("bootstrap state should load")
            .expect("bootstrap password should exist");
        let canonical = mgr
            .unlock(&bootstrap_password)
            .expect("bootstrap password should unlock");
        let ciphertext = canonical.encrypt(b"bootstrap-race-roundtrip").unwrap();

        for key in keys {
            let decrypted = key
                .decrypt(&ciphertext)
                .expect("all callers should share one key");
            assert_eq!(&decrypted[..], b"bootstrap-race-roundtrip");
        }
    }

    #[test]
    fn install_managed_secret_initializes_without_keyfile_bootstrap() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = MasterPasswordManager::new(tmp.path(), tmp.path());

        let key = mgr
            .initialize_startup_password_if_needed("install-secret")
            .unwrap();

        assert!(mgr.is_password_set());
        assert!(!mgr.is_bootstrap_password_active().unwrap());
        assert!(mgr.is_install_managed_password_active().unwrap());
        assert!(!tmp.path().join(".keyfile").exists());

        let unlocked = mgr.unlock("install-secret").unwrap();
        let ciphertext = key.encrypt(b"install-managed").unwrap();
        let decrypted = unlocked.decrypt(&ciphertext).unwrap();
        assert_eq!(&decrypted[..], b"install-managed");
    }

    #[test]
    fn install_managed_initialization_is_atomic_across_concurrent_callers() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = std::sync::Arc::new(MasterPasswordManager::new(tmp.path(), tmp.path()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));

        let handles = (0..8)
            .map(|_| {
                let mgr = mgr.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    mgr.initialize_startup_password_if_needed("install-secret")
                        .expect("install-managed init should succeed")
                })
            })
            .collect::<Vec<_>>();

        let keys = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread should complete"))
            .collect::<Vec<_>>();

        assert!(mgr.is_install_managed_password_active().unwrap());
        assert!(!tmp.path().join(".keyfile").exists());
        let canonical = mgr
            .unlock("install-secret")
            .expect("install secret should unlock");
        let ciphertext = canonical.encrypt(b"install-race-roundtrip").unwrap();

        for key in keys {
            let decrypted = key
                .decrypt(&ciphertext)
                .expect("all callers should share one key");
            assert_eq!(&decrypted[..], b"install-race-roundtrip");
        }
    }

    #[test]
    fn test_remove_password() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = MasterPasswordManager::new(tmp.path(), tmp.path());

        let prepared = mgr.prepare_password("my-password").unwrap();
        mgr.commit_prepared_password(prepared).unwrap();
        assert!(mgr.is_password_set());

        let _new_key = mgr.remove_password().unwrap();
        assert!(!mgr.is_password_set());
    }
}
