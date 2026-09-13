//! emlnet — a multi-writer CRDT store replicated over TCP.
//!
//! Each op is `{id, writer, ts, key, value}`; state is a per-key
//! last-writer-wins register ordered by `(ts, writer)`. Ops are idempotent
//! (deduplicated by `id`), so any number of syncs converges.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Op {
    pub id: String,
    pub writer: String,
    pub ts: u64,
    pub key: String,
    pub value: serde_json::Value,
}

pub struct Store {
    dir: PathBuf,
    ops: Vec<Op>,
    seen: HashSet<String>,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn op_id(writer: &str, ts: u64, key: &str, value: &serde_json::Value) -> String {
    let mut h = Sha256::new();
    h.update(writer.as_bytes());
    h.update(b"|");
    h.update(ts.to_string().as_bytes());
    h.update(b"|");
    h.update(key.as_bytes());
    h.update(b"|");
    h.update(serde_json::to_vec(value).unwrap_or_default());
    let d = h.finalize();
    let mut s = String::new();
    for b in &d[..8] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

impl Store {
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut s = Store {
            dir,
            ops: Vec::new(),
            seen: HashSet::new(),
        };
        s.load()?;
        Ok(s)
    }

    fn log_path(&self) -> PathBuf {
        self.dir.join("ops.jsonl")
    }

    fn load(&mut self) -> io::Result<()> {
        let p = self.log_path();
        if !p.exists() {
            return Ok(());
        }
        let f = fs::File::open(p)?;
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(op) = serde_json::from_str::<Op>(&line) {
                if self.seen.insert(op.id.clone()) {
                    self.ops.push(op);
                }
            }
        }
        Ok(())
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Local write: append an op and persist it.
    pub fn put(&mut self, writer: &str, key: &str, value: serde_json::Value) -> io::Result<Op> {
        let ts = now_ts();
        let id = op_id(writer, ts, key, &value);
        let op = Op {
            id,
            writer: writer.to_string(),
            ts,
            key: key.to_string(),
            value,
        };
        self.append(&op)?;
        Ok(op)
    }

    fn append(&mut self, op: &Op) -> io::Result<()> {
        if !self.seen.insert(op.id.clone()) {
            return Ok(());
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        writeln!(f, "{}", serde_json::to_string(op).unwrap())?;
        self.ops.push(op.clone());
        Ok(())
    }
}

impl Store {
    /// Merge remote ops (idempotent). Returns how many were new.
    pub fn merge(&mut self, ops: Vec<Op>) -> io::Result<usize> {
        let before = self.ops.len();
        for op in ops {
            self.append(&op)?;
        }
        Ok(self.ops.len() - before)
    }

    /// Materialised LWW state: key -> value, ordered by (ts, writer).
    pub fn state(&self) -> BTreeMap<String, serde_json::Value> {
        let mut best: BTreeMap<String, (u64, String)> = BTreeMap::new();
        let mut out = BTreeMap::new();
        for op in &self.ops {
            let cand = (op.ts, op.writer.clone());
            match best.get(&op.key) {
                Some(cur) if *cur >= cand => {}
                _ => {
                    best.insert(op.key.clone(), cand);
                    out.insert(op.key.clone(), op.value.clone());
                }
            }
        }
        out
    }

    pub fn state_of(&self, key: &str) -> Option<serde_json::Value> {
        self.state().get(key).cloned()
    }

    /// Accept one inbound sync: read peer ops, merge, then send ours back.
    pub fn serve_once(&mut self, listener: &TcpListener) -> io::Result<()> {
        let (sock, _) = listener.accept()?;
        self.exchange(sock)
    }

    /// Bidirectional exchange on an established connection.
    pub fn exchange(&mut self, mut sock: TcpStream) -> io::Result<()> {
        let reader_sock = sock.try_clone()?;
        let mut reader = BufReader::new(reader_sock);
        let mut incoming = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            if let Ok(op) = serde_json::from_str::<Op>(line.trim()) {
                incoming.push(op);
            }
        }
        self.merge(incoming)?;
        for op in &self.ops {
            writeln!(sock, "{}", serde_json::to_string(op).unwrap())?;
        }
        sock.flush()?;
        Ok(())
    }

    /// Client side: send all our ops, then read the peer's ops.
    pub fn sync_to(&mut self, peer: &str) -> io::Result<usize> {
        let mut sock = TcpStream::connect(peer)?;
        for op in &self.ops {
            writeln!(sock, "{}", serde_json::to_string(op).unwrap())?;
        }
        sock.flush()?;
        sock.shutdown(std::net::Shutdown::Write)?;
        let mut reader = BufReader::new(sock);
        let mut incoming = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            if let Ok(op) = serde_json::from_str::<Op>(line.trim()) {
                incoming.push(op);
            }
        }
        self.merge(incoming)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emlnet_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn lww_and_idempotent_merge() {
        let dir = tmp("lww");
        let mut s = Store::open(&dir).unwrap();
        s.put("w1", "x", serde_json::json!(1)).unwrap();
        s.put("w1", "x", serde_json::json!(2)).unwrap();
        assert_eq!(s.state_of("x"), Some(serde_json::json!(2)));
        let ops = s.ops().to_vec();
        let added = s.merge(ops).unwrap();
        assert_eq!(added, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_stores_converge_over_tcp() {
        let a = tmp("convA");
        let b = tmp("convB");
        let mut sa = Store::open(&a).unwrap();
        let mut sb = Store::open(&b).unwrap();
        sa.put("A", "x", serde_json::json!(1)).unwrap();
        sb.put("B", "y", serde_json::json!(2)).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            sa.serve_once(&listener).unwrap();
            sa
        });
        sb.sync_to(&format!("{}", addr)).unwrap();
        let sa = server.join().unwrap();

        // both sides now hold the union
        assert_eq!(sa.state_of("x"), Some(serde_json::json!(1)));
        assert_eq!(sa.state_of("y"), Some(serde_json::json!(2)));
        assert_eq!(sb.state_of("x"), Some(serde_json::json!(1)));
        assert_eq!(sb.state_of("y"), Some(serde_json::json!(2)));
        assert_eq!(sa.len(), 2);
        assert_eq!(sb.len(), 2);
        // and states are equal
        assert_eq!(sa.state(), sb.state());
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tmp("persist");
        {
            let mut s = Store::open(&dir).unwrap();
            s.put("w", "k", serde_json::json!("v")).unwrap();
        }
        let s2 = Store::open(&dir).unwrap();
        assert_eq!(s2.state_of("k"), Some(serde_json::json!("v")));
        let _ = fs::remove_dir_all(&dir);
    }
}
