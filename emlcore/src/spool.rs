//! Append-only, hash-chained spool of event `.eml` files (the bus and the log).

use crate::chain::{link_hash, GENESIS};
use crate::record::Record;
use crate::time::now_rfc3339;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct Event {
    pub seq: u64,
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub source: String,
    pub payload: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainStatus {
    pub ok: bool,
    pub checked: usize,
    pub broken_at: Option<u64>,
}

pub struct Spool {
    dir: PathBuf,
    events: Vec<Event>,
    head: String,
    seq: u64,
    counter: AtomicU64,
}

impl Spool {
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut s = Spool {
            dir,
            events: Vec::new(),
            head: GENESIS.to_string(),
            seq: 0,
            counter: AtomicU64::new(0),
        };
        s.load();
        Ok(s)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn head(&self) -> &str {
        &self.head
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Append one event: render -> tmp -> atomic rename.
    pub fn append(
        &mut self,
        kind: &str,
        source: &str,
        payload: serde_json::Value,
    ) -> io::Result<Event> {
        let seq = self.seq + 1;
        let ts = now_rfc3339();
        let id = self.gen_id(seq);
        let body = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
        let prev = self.head.clone();
        let hash = link_hash(&prev, seq, &id, &ts, kind, &body);

        let mut rec = Record::new();
        rec.set("From", format!("<{}@eml.local>", source));
        rec.set("To", "<eml@localhost>");
        rec.set("X-EMLBox-Msg", "v1");
        rec.set("X-Event", kind);
        rec.set("X-Emllog-Kind", kind);
        rec.set("X-Emllog-Seq", seq.to_string());
        rec.set("X-Emllog-Source", source);
        rec.set("X-Emllog-Prev-Hash", prev.clone());
        rec.set("X-Emllog-Hash", hash.clone());
        rec.set("Message-ID", format!("<{}@eml>", id));
        rec.set("Date", ts.clone());
        rec.set("Content-Type", "application/json; charset=utf-8");
        let mut b = body.clone();
        if !b.ends_with(b"\n") {
            b.push(b'\n');
        }
        rec.body = b;

        let fname = format!("{:08}.{}.msg.eml", seq, id);
        let tmp = self.dir.join(format!("{}.tmp", fname));
        fs::write(&tmp, rec.render())?;
        fs::rename(&tmp, self.dir.join(&fname))?;

        let ev = Event {
            seq,
            id,
            ts,
            kind: kind.to_string(),
            source: source.to_string(),
            payload,
            prev_hash: prev,
            hash: hash.clone(),
            path: self.dir.join(&fname),
        };
        self.seq = seq;
        self.head = hash;
        self.events.push(ev.clone());
        Ok(ev)
    }

    fn gen_id(&self, seq: u64) -> String {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let c = self.counter.fetch_add(1, Ordering::Relaxed) as u128;
        let pid = std::process::id() as u128;
        let x = n ^ (c << 40) ^ (pid << 16) ^ seq as u128;
        format!("{:016x}", x)
    }

    fn load(&mut self) {
        let mut entries: Vec<PathBuf> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "eml").unwrap_or(false))
            .collect();
        entries.sort();
        let mut head = GENESIS.to_string();
        let mut seq = 0u64;
        for p in entries {
            if let Ok(text) = fs::read(&p) {
                if let Some(ev) = parse_event(&text, &p) {
                    head = ev.hash.clone();
                    seq = ev.seq;
                    self.events.push(ev);
                }
            }
        }
        self.head = head;
        self.seq = seq;
    }

    /// Recompute the chain. Returns where it first breaks (if anywhere).
    pub fn verify(&self) -> ChainStatus {
        let mut prev = GENESIS.to_string();
        for (i, e) in self.events.iter().enumerate() {
            let body = serde_json::to_vec(&e.payload).unwrap_or_default();
            let expect = link_hash(&prev, e.seq, &e.id, &e.ts, &e.kind, &body);
            if e.prev_hash != prev || e.hash != expect {
                return ChainStatus {
                    ok: false,
                    checked: i,
                    broken_at: Some(e.seq),
                };
            }
            prev = e.hash.clone();
        }
        ChainStatus {
            ok: true,
            checked: self.events.len(),
            broken_at: None,
        }
    }
}

fn parse_event(data: &[u8], path: &Path) -> Option<Event> {
    let rec = Record::parse(data);
    let seq = rec.get("X-Emllog-Seq")?.parse().ok()?;
    let kind = rec
        .get("X-Event")
        .or_else(|| rec.get("X-Emllog-Kind"))?
        .to_string();
    let payload: serde_json::Value = serde_json::from_slice(&rec.body).ok()?;
    let raw_id = rec.get("Message-ID").unwrap_or("<x@eml>");
    let id = raw_id
        .trim_matches(|c| c == '<' || c == '>')
        .split('@')
        .next()
        .unwrap_or("x")
        .to_string();
    Some(Event {
        seq,
        id,
        ts: rec.get("Date").unwrap_or("").to_string(),
        kind,
        source: rec.get("X-Emllog-Source").unwrap_or("unknown").to_string(),
        payload,
        prev_hash: rec.get("X-Emllog-Prev-Hash").unwrap_or("").to_string(),
        hash: rec.get("X-Emllog-Hash").unwrap_or("").to_string(),
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_verify_chain() {
        let dir = std::env::temp_dir().join(format!("emlcore_spool_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        {
            let mut s = Spool::open(&dir).unwrap();
            s.append("INIT", "test", serde_json::json!({"a": 1})).unwrap();
            s.append("START", "test", serde_json::json!({"b": 2})).unwrap();
            assert_eq!(s.len(), 2);
            assert!(s.verify().ok);
        }
        // reopen: chain must survive a fresh load
        let s2 = Spool::open(&dir).unwrap();
        assert_eq!(s2.len(), 2);
        assert!(s2.verify().ok);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tamper_is_detected() {
        let dir = std::env::temp_dir().join(format!("emlcore_tamper_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = Spool::open(&dir).unwrap();
        s.append("A", "test", serde_json::json!({"x": 1})).unwrap();
        s.append("B", "test", serde_json::json!({"x": 2})).unwrap();
        let victim = s.events()[1].path.clone();
        let text = fs::read_to_string(&victim).unwrap().replace("\"x\":2", "\"x\":9");
        fs::write(&victim, text).unwrap();
        let s2 = Spool::open(&dir).unwrap();
        assert!(!s2.verify().ok);
        let _ = fs::remove_dir_all(&dir);
    }
}
