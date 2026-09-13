//! SHA-256 hash-chain over events. Every event commits to its predecessor.

use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000";

/// 16-hex-char (64-bit) link digest: `H(prev | seq | id | ts | kind | body)`.
pub fn link_hash(prev: &str, seq: u64, id: &str, ts: &str, kind: &str, body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(b"|");
    h.update(seq.to_string().as_bytes());
    h.update(b"|");
    h.update(id.as_bytes());
    h.update(b"|");
    h.update(ts.as_bytes());
    h.update(b"|");
    h.update(kind.as_bytes());
    h.update(b"|");
    h.update(body);
    hex16(&h.finalize())
}

pub fn hex16(digest: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    for b in &digest[..8] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_sensitive() {
        let a = link_hash(GENESIS, 1, "abc", "t", "START", b"{}");
        let b = link_hash(GENESIS, 1, "abc", "t", "START", b"{}");
        let c = link_hash(GENESIS, 1, "abc", "t", "STOP", b"{}");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
    }
}
