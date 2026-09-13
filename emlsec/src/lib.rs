//! emlsec — secrets as AES-256-GCM encrypted `.eml` records.
//!
//! Ciphertext is hex-encoded into the record body (so the RFC 822 renderer
//! never touches binary); the 12-byte nonce lives in `X-Sec-Nonce`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use emlcore::Record;
use rand::RngCore;

pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("odd hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Encrypt with a fresh random nonce. Returns (nonce, ciphertext).
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<([u8; 12], Vec<u8>), String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| e.to_string())?;
    Ok((nonce, ct))
}

pub fn decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if nonce.len() != 12 {
        return Err("nonce must be 12 bytes".into());
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| e.to_string())
}

/// Build a secret record (hex-encoded ciphertext in the body).
pub fn secret_record(name: &str, key: &[u8; 32], plaintext: &[u8]) -> Result<Record, String> {
    let (nonce, ct) = encrypt(key, plaintext)?;
    let mut rec = Record::new();
    rec.set("From", "<emlsec@eml.local>");
    rec.set("To", "<vault@eml.local>");
    rec.set("Subject", name);
    rec.set("X-EML-Type", "Application/Secret");
    rec.set("X-Entity-ID", name);
    rec.set("X-Sec-Alg", "aes-256-gcm");
    rec.set("X-Sec-Nonce", to_hex(&nonce));
    rec.set("Content-Type", "application/x-emlsec-hex");
    rec.body = format!("{}\n", to_hex(&ct)).into_bytes();
    Ok(rec)
}

/// Decrypt a secret record.
pub fn open_record(rec: &Record, key: &[u8; 32]) -> Result<Vec<u8>, String> {
    let alg = rec.get("X-Sec-Alg").ok_or("not a secret record")?;
    if alg != "aes-256-gcm" {
        return Err(format!("unsupported alg: {}", alg));
    }
    let nonce = from_hex(rec.get("X-Sec-Nonce").ok_or("missing X-Sec-Nonce")?)?;
    let ct = from_hex(&String::from_utf8_lossy(&rec.body))?;
    decrypt(key, &nonce, &ct)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [42u8; 32];

    #[test]
    fn roundtrip() {
        let rec = secret_record("db/password", &KEY, b"s3cr3t-value").unwrap();
        let got = open_record(&rec, &KEY).unwrap();
        assert_eq!(got, b"s3cr3t-value");
    }

    #[test]
    fn wrong_key_fails() {
        let rec = secret_record("k", &KEY, b"top").unwrap();
        let other = [7u8; 32];
        assert!(open_record(&rec, &other).is_err());
    }

    #[test]
    fn tamper_fails() {
        let mut rec = secret_record("k", &KEY, b"top").unwrap();
        // flip one ciphertext nibble
        let body = String::from_utf8_lossy(&rec.body).trim().to_string();
        let mut chars: Vec<char> = body.chars().collect();
        chars[0] = if chars[0] == '0' { '1' } else { '0' };
        rec.body = format!("{}\n", chars.into_iter().collect::<String>()).into_bytes();
        assert!(open_record(&rec, &KEY).is_err());
    }

    #[test]
    fn nonce_is_random() {
        let a = secret_record("k", &KEY, b"same").unwrap();
        let b = secret_record("k", &KEY, b"same").unwrap();
        assert_ne!(a.get("X-Sec-Nonce"), b.get("X-Sec-Nonce"));
    }
}
