//! emlfs — an in-memory tree over a directory of `.eml` containers.
//!
//! Each `<name>.eml` becomes a directory `<name>/` with read-only entries:
//! `raw` (whole file), `headers` (text), `body` (bytes), and `payload.json`
//! when the body is JSON.

use emlcore::Record;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::Path;

pub enum Node {
    Dir(BTreeMap<String, u64>),
    File(Vec<u8>),
}

pub struct Tree {
    pub nodes: HashMap<u64, Node>,
    next: u64,
}

pub const ROOT: u64 = 1;

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(ROOT, Node::Dir(BTreeMap::new()));
        Tree { nodes, next: 2 }
    }

    pub fn get(&self, ino: u64) -> Option<&Node> {
        self.nodes.get(&ino)
    }

    pub fn lookup(&self, parent: u64, name: &str) -> Option<u64> {
        match self.nodes.get(&parent) {
            Some(Node::Dir(children)) => children.get(name).copied(),
            _ => None,
        }
    }

    pub fn children(&self, ino: u64) -> Option<&BTreeMap<String, u64>> {
        match self.nodes.get(&ino) {
            Some(Node::Dir(c)) => Some(c),
            _ => None,
        }
    }

    fn add_dir(&mut self, parent: u64, name: &str) -> u64 {
        let ino = self.next;
        self.next += 1;
        self.nodes.insert(ino, Node::Dir(BTreeMap::new()));
        if let Some(Node::Dir(c)) = self.nodes.get_mut(&parent) {
            c.insert(name.to_string(), ino);
        }
        ino
    }

    fn add_file(&mut self, parent: u64, name: &str, data: Vec<u8>) -> u64 {
        let ino = self.next;
        self.next += 1;
        self.nodes.insert(ino, Node::File(data));
        if let Some(Node::Dir(c)) = self.nodes.get_mut(&parent) {
            c.insert(name.to_string(), ino);
        }
        ino
    }

    /// Build a tree from every `*.eml` file in `store`.
    pub fn build_store(store: &Path) -> io::Result<Tree> {
        let mut tree = Tree::new();
        let mut files: Vec<_> = fs::read_dir(store)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "eml").unwrap_or(false))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unnamed".into());
            let data = fs::read(&path)?;
            let rec = Record::parse(&data);
            let dir = tree.add_dir(ROOT, &name);
            tree.add_file(dir, "raw", data.clone());
            let headers: String = rec
                .headers
                .iter()
                .map(|(k, v)| format!("{}: {}\n", k, v))
                .collect();
            tree.add_file(dir, "headers", headers.into_bytes());
            tree.add_file(dir, "body", rec.body.clone());
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&rec.body) {
                let pretty = serde_json::to_vec_pretty(&v).unwrap_or_default();
                tree.add_file(dir, "payload.json", pretty);
            }
        }
        Ok(tree)
    }

    pub fn inode_count(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tree_from_store() {
        let dir = std::env::temp_dir().join(format!("emlfs_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("svc.eml"),
            "From: <a@b>\nX-Event: START\nContent-Type: application/json\n\n{\"unit\":\"svc\"}\n",
        )
        .unwrap();

        let tree = Tree::build_store(&dir).unwrap();
        let svc = tree.lookup(ROOT, "svc").expect("svc dir");
        assert!(matches!(tree.get(svc), Some(Node::Dir(_))));
        assert!(tree.lookup(svc, "raw").is_some());
        assert!(tree.lookup(svc, "headers").is_some());
        let body = tree.lookup(svc, "body").unwrap();
        match tree.get(body).unwrap() {
            Node::File(d) => assert!(String::from_utf8_lossy(d).contains("svc")),
            _ => panic!("body not a file"),
        }
        assert!(tree.lookup(svc, "payload.json").is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
