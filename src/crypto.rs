use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    pub fn from_env() -> Result<Self, String> {
        let source = crate::secret_env::required("MASTER_KEY")?;
        if source.len() < 32 {
            return Err("MASTER_KEY must contain at least 32 characters".to_owned());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&Sha256::digest(source.as_bytes()));
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(&Nonce::from(nonce), plaintext.as_bytes())
            .map_err(|_| "encryption failed".to_owned())?;
        Ok(format!("{}:{}", B64.encode(nonce), B64.encode(ciphertext)))
    }

    pub fn decrypt(&self, encoded: &str) -> Result<String, String> {
        let (nonce, ciphertext) = encoded
            .split_once(':')
            .ok_or_else(|| "invalid ciphertext".to_owned())?;
        let nonce = B64.decode(nonce).map_err(|_| "invalid nonce".to_owned())?;
        let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| "invalid nonce".to_owned())?;
        let ciphertext = B64
            .decode(ciphertext)
            .map_err(|_| "invalid ciphertext".to_owned())?;
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|e| e.to_string())?;
        let plaintext = cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| "decryption failed".to_owned())?;
        String::from_utf8(plaintext).map_err(|_| "decrypted value is not utf8".to_owned())
    }
}
