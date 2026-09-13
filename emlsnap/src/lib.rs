//! emlsnap — snapshot a `.eml` store into a verifiable bundle.
//!
//! A snapshot is a copy of every file plus `manifest.eml` (an RFC 822 record
//! whose JSON body lists each file with its SHA-256). `verify` recomputes the
//! digests; `restore` copies the files back.

use emlcore::Record;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "manifest.eml";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in &d {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn list_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.is_file() {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name != MANIFEST {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Copy `src` into `snap` and write a hashed manifest. Returns file count.
pub fn snapshot(src: &Path, snap: &Path) -> Result<usize, String> {
    fs::create_dir_all(snap).map_err(|e| e.to_string())?;
    let files = list_files(src)?;
    let mut entries = Vec::new();
    for p in &files {
        let data = fs::read(p).map_err(|e| e.to_string())?;
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        fs::write(snap.join(&name), &data).map_err(|e| e.to_string())?;
        entries.push(Entry {
            path: name,
            sha256: sha256_hex(&data),
            size: data.len() as u64,
        });
    }
    let mut rec = Record::new();
    rec.set("From", "<emlsnap@eml.local>");
    rec.set("To", "<backup@eml.local>");
    rec.set("Subject", "snapshot manifest");
    rec.set("X-EML-Type", "Application/Snapshot");
    rec.set("Content-Type", "application/json");
    rec.body = serde_json::to_vec(&entries).map_err(|e| e.to_string())?;
    fs::write(snap.join(MANIFEST), rec.render()).map_err(|e| e.to_string())?;
    Ok(entries.len())
}

fn read_manifest(snap: &Path) -> Result<Vec<Entry>, String> {
    let data = fs::read(snap.join(MANIFEST)).map_err(|e| e.to_string())?;
    let rec = Record::parse(&data);
    serde_json::from_slice(&rec.body).map_err(|e| e.to_string())
}

/// Recompute digests. Returns a list of problems (empty == intact).
pub fn verify(snap: &Path) -> Result<Vec<String>, String> {
    let entries = read_manifest(snap)?;
    let mut problems = Vec::new();
    for e in entries {
        let p = snap.join(&e.path);
        match fs::read(&p) {
            Ok(data) => {
                let got = sha256_hex(&data);
                if got != e.sha256 {
                    problems.push(format!("{}: hash mismatch", e.path));
                }
                if data.len() as u64 != e.size {
                    problems.push(format!("{}: size mismatch", e.path));
                }
            }
            Err(_) => problems.push(format!("{}: missing", e.path)),
        }
    }
    Ok(problems)
}

/// Restore snapshot files into `out`. Returns file count.
pub fn restore(snap: &Path, out: &Path) -> Result<usize, String> {
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let entries = read_manifest(snap)?;
    let mut n = 0;
    for e in entries {
        let data = fs::read(snap.join(&e.path)).map_err(|e| e.to_string())?;
        if sha256_hex(&data) != e.sha256 {
            return Err(format!("{}: snapshot corrupted", e.path));
        }
        fs::write(out.join(&e.path), &data).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emlsnap_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn snapshot_verify_restore() {
        let src = tmp("src");
        let snap = tmp("snap");
        let out = tmp("out");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.eml"), "From: <x>\n\nhello\n").unwrap();
        fs::write(src.join("b.eml"), "From: <y>\n\nworld\n").unwrap();

        assert_eq!(snapshot(&src, &snap).unwrap(), 2);
        assert!(verify(&snap).unwrap().is_empty());

        // tamper inside the snapshot -> verify reports it
        fs::write(snap.join("a.eml"), "tampered").unwrap();
        let problems = verify(&snap).unwrap();
        assert!(problems.iter().any(|p| p.contains("hash mismatch")));

        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&snap);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn restore_roundtrip() {
        let src = tmp("rsrc");
        let snap = tmp("rsnap");
        let out = tmp("rout");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("k.eml"), "payload-1").unwrap();
        snapshot(&src, &snap).unwrap();
        assert_eq!(restore(&snap, &out).unwrap(), 1);
        assert_eq!(fs::read_to_string(out.join("k.eml")).unwrap(), "payload-1");
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&snap);
        let _ = fs::remove_dir_all(&out);
    }
}
