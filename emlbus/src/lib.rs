//! emlbus — a publish/subscribe bus where every message is a `.eml` file.
//!
//! Delivery is by the `To:` header (an entity id, a `topic:name`, or `*`).
//! Consumed messages are renamed `.done`; nothing is ever destroyed.

use emlcore::{Record, Spool};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct Bus {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub path: PathBuf,
    pub from: String,
    pub to: String,
    pub event: String,
    pub body: serde_json::Value,
}

impl Bus {
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(Bus { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Publish a message; atomic write via tmp + rename.
    pub fn publish(
        &self,
        from: &str,
        to: &str,
        event: &str,
        body: serde_json::Value,
    ) -> io::Result<PathBuf> {
        let mut rec = Record::new();
        rec.set("From", format!("<{}@eml.local>", from));
        rec.set("To", format!("<{}>", to));
        rec.set("X-EMLBox-Msg", "v1");
        rec.set("X-Event", event);
        rec.set("X-Bus-From", from);
        rec.set("X-Bus-To", to);
        rec.set("Content-Type", "application/json");
        let mut b = serde_json::to_vec(&body).unwrap_or_default();
        b.push(b'\n');
        rec.body = b;

        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("{:016x}.msg.eml", n);
        let tmp = self.dir.join(format!("{}.tmp", name));
        fs::write(&tmp, rec.render())?;
        let dest = self.dir.join(&name);
        fs::rename(&tmp, &dest)?;
        Ok(dest)
    }

    /// All not-yet-consumed messages matching `to` (`*` matches anything).
    pub fn pending(&self, to: &str) -> Vec<Message> {
        let mut out = Vec::new();
        let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "eml").unwrap_or(false))
            .collect();
        files.sort();
        for p in files {
            if let Some(m) = parse(&p) {
                if matches(&m.to, to) {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Take the oldest matching message, marking it consumed (`.done`).
    pub fn consume(&self, to: &str) -> Option<Message> {
        let m = self.pending(to).into_iter().next()?;
        let _ = fs::rename(&m.path, m.path.with_extension("eml.done"));
        Some(m)
    }

    pub fn len(&self) -> usize {
        fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "eml").unwrap_or(false))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convenience: open a spool view (for hash-chained logs, optional).
    pub fn spool(&self) -> io::Result<Spool> {
        Spool::open(&self.dir)
    }
}

fn parse(p: &Path) -> Option<Message> {
    let data = fs::read(p).ok()?;
    let rec = Record::parse(&data);
    let body = serde_json::from_slice(&rec.body).unwrap_or(serde_json::Value::Null);
    Some(Message {
        path: p.to_path_buf(),
        from: rec.get("X-Bus-From").unwrap_or("").to_string(),
        to: rec.get("X-Bus-To").unwrap_or("").to_string(),
        event: rec.get("X-Event").unwrap_or("").to_string(),
        body,
    })
}

/// `*` matches everything; otherwise exact match (case-insensitive).
pub fn matches(actual: &str, filter: &str) -> bool {
    filter == "*" || actual.eq_ignore_ascii_case(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emlbus_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn publish_and_consume_by_target() {
        let dir = tmp("pubsub");
        let bus = Bus::open(&dir).unwrap();
        bus.publish("a", "worker", "JOB", serde_json::json!({"n": 1})).unwrap();
        bus.publish("a", "other", "JOB", serde_json::json!({"n": 2})).unwrap();

        assert_eq!(bus.pending("worker").len(), 1);
        let m = bus.consume("worker").unwrap();
        assert_eq!(m.event, "JOB");
        assert_eq!(m.body["n"], 1);
        // consumed: no longer pending
        assert_eq!(bus.pending("worker").len(), 0);
        // other target untouched
        assert_eq!(bus.pending("other").len(), 1);
        // wildcard sees remaining
        assert_eq!(bus.pending("*").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wildcard_matches_all() {
        assert!(matches("worker", "*"));
        assert!(matches("WORKER", "worker"));
        assert!(!matches("worker", "other"));
    }
}
