//! emlsign — ed25519 provenance for `.eml` records.
//!
//! Signing canonicalises the record (all `X-Sign*` headers stripped, then the
//! RFC 822 bytes), so a signature covers headers *and* body. Verification
//! recomputes the same bytes and checks the detached ed25519 signature.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use emlcore::Record;

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

/// Canonical bytes: the record without any `X-Sign*` headers.
pub fn canonical(rec: &Record) -> Vec<u8> {
    let mut r = rec.clone();
    r.headers
        .retain(|(k, _)| !k.to_ascii_lowercase().starts_with("x-sign"));
    r.render()
}

pub fn generate() -> SigningKey {
    SigningKey::generate(&mut rand::rngs::OsRng)
}

pub fn sign_record(sk: &SigningKey, rec: &mut Record) {
    let msg = canonical(rec);
    let sig = sk.sign(&msg);
    rec.set("X-Sign-Alg", "ed25519");
    rec.set("X-Sign-Pub", to_hex(sk.verifying_key().as_bytes()));
    rec.set("X-Sign", to_hex(&sig.to_bytes()));
}

/// Verify a signed record. If `expect_pub` is given, the embedded public key
/// must match it (pin the signer identity).
pub fn verify_record(rec: &Record, expect_pub: Option<[u8; 32]>) -> Result<(), String> {
    let alg = rec.get("X-Sign-Alg").ok_or("missing X-Sign-Alg")?;
    if alg != "ed25519" {
        return Err(format!("unsupported alg: {}", alg));
    }
    let pub_hex = rec.get("X-Sign-Pub").ok_or("missing X-Sign-Pub")?;
    let sig_hex = rec.get("X-Sign").ok_or("missing X-Sign")?;

    let pub_bytes = from_hex(pub_hex)?;
    if pub_bytes.len() != 32 {
        return Err("bad public key length".into());
    }
    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes);
    if let Some(exp) = expect_pub {
        if exp != pub_arr {
            return Err("public key does not match pinned signer".into());
        }
    }
    let vk = VerifyingKey::from_bytes(&pub_arr).map_err(|e| e.to_string())?;

    let sig_bytes = from_hex(sig_hex)?;
    if sig_bytes.len() != 64 {
        return Err("bad signature length".into());
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    let msg = canonical(rec);
    vk.verify_strict(&msg, &sig).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Record {
        let mut r = Record::new();
        r.set("From", "<a@eml.local>");
        r.set("To", "<b@eml.local>");
        r.set("X-Event", "START");
        r.body = b"{\"unit\":\"web\"}\n".to_vec();
        r
    }

    #[test]
    fn sign_and_verify() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let mut r = rec();
        sign_record(&sk, &mut r);
        assert!(verify_record(&r, None).is_ok());
        assert!(verify_record(&r, Some(sk.verifying_key().to_bytes())).is_ok());
    }

    #[test]
    fn tamper_breaks_signature() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let mut r = rec();
        sign_record(&sk, &mut r);
        r.body = b"{\"unit\":\"EVIL\"}\n".to_vec();
        assert!(verify_record(&r, None).is_err());
    }

    #[test]
    fn header_tamper_breaks_signature() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let mut r = rec();
        sign_record(&sk, &mut r);
        r.set("X-Event", "STOP");
        assert!(verify_record(&r, None).is_err());
    }

    #[test]
    fn pinned_key_rejects_other_signer() {
        let sk1 = SigningKey::from_bytes(&[1u8; 32]);
        let sk2 = SigningKey::from_bytes(&[2u8; 32]);
        let mut r = rec();
        sign_record(&sk1, &mut r);
        assert!(verify_record(&r, Some(sk2.verifying_key().to_bytes())).is_err());
    }
}
