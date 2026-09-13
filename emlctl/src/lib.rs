//! emlctl — discovery and health of `.eml` spools across a directory tree.

use emlcore::Spool;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SpoolInfo {
    pub dir: PathBuf,
    pub events: usize,
    pub chain_ok: bool,
    pub head: String,
}

fn is_spool(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path()
                    .to_string_lossy()
                    .ends_with(".msg.eml")
            })
        })
        .unwrap_or(false)
}

fn collect(dir: &Path, depth: usize, max: usize, out: &mut Vec<SpoolInfo>) {
    if depth > max {
        return;
    }
    if is_spool(dir) {
        if let Ok(s) = Spool::open(dir) {
            let st = s.verify();
            out.push(SpoolInfo {
                dir: dir.to_path_buf(),
                events: s.len(),
                chain_ok: st.ok,
                head: s.head().to_string(),
            });
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, depth + 1, max, out);
            }
        }
    }
}

/// Find all spools under `root` (including `root`), up to `max_depth`.
pub fn scan_spools(root: &Path, max_depth: usize) -> Vec<SpoolInfo> {
    let mut out = Vec::new();
    collect(root, 0, max_depth, &mut out);
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_spools() {
        let root = std::env::temp_dir().join(format!("emlctl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("run/emlinit");
        let b = root.join("run/emlcron");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        {
            let mut s = Spool::open(&a).unwrap();
            s.append("START", "t", serde_json::json!({"x": 1})).unwrap();
        }
        {
            let mut s = Spool::open(&b).unwrap();
            s.append("RUN", "t", serde_json::json!({"y": 1})).unwrap();
            s.append("EXIT", "t", serde_json::json!({"y": 2})).unwrap();
        }
        let found = scan_spools(&root, 3);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|s| s.chain_ok));
        let total: usize = found.iter().map(|s| s.events).sum();
        assert_eq!(total, 3);
        let _ = std::fs::remove_dir_all(&root);
    }
}
