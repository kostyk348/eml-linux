//! emlfs — mount a directory of `.eml` containers as a read-only filesystem.
//!
//!   emlfs <store-dir> <mountpoint>
//!
//! Each `<name>.eml` appears as `<name>/` containing `raw`, `headers`, `body`
//! and (when JSON) `payload.json`. Read-only; unmount with `fusermount3 -u`.

use emlfs::{Node, Tree, ROOT};
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, Request,
};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime};

const TTL: Duration = Duration::from_secs(1);

struct EmlFs {
    tree: Tree,
}

impl EmlFs {
    fn attr(&self, ino: u64) -> Option<FileAttr> {
        let node = self.tree.get(ino)?;
        let (kind, size, perm, nlink) = match node {
            Node::Dir(c) => (FileType::Directory, 0u64, 0o555, 2 + c.len() as u32),
            Node::File(d) => (FileType::RegularFile, d.len() as u64, 0o444, 1),
        };
        let now = SystemTime::now();
        Some(FileAttr {
            ino,
            size,
            blocks: (size + 511) / 512,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind,
            perm,
            nlink,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 512,
            flags: 0,
        })
    }
}

impl Filesystem for EmlFs {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match self.tree.lookup(parent, &name.to_string_lossy()) {
            Some(ino) => match self.attr(ino) {
                Some(a) => reply.entry(&TTL, &a, 0),
                None => reply.error(libc::ENOENT),
            },
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        match self.attr(ino) {
            Some(a) => reply.attr(&TTL, &a),
            None => reply.error(libc::ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        match self.tree.get(ino) {
            Some(Node::File(d)) => {
                let start = offset.max(0) as usize;
                if start >= d.len() {
                    reply.data(&[]);
                } else {
                    let end = (start + size as usize).min(d.len());
                    reply.data(&d[start..end]);
                }
            }
            _ => reply.error(libc::EISDIR),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        match self.tree.get(ino) {
            Some(Node::File(_)) => reply.opened(0, 0),
            _ => reply.error(libc::EISDIR),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let children = match self.tree.children(ino) {
            Some(c) => c,
            None => {
                reply.error(libc::ENOTDIR);
                return;
            }
        };
        let mut entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ROOT, FileType::Directory, "..".to_string()),
        ];
        for (name, cino) in children {
            let kind = match self.tree.get(*cino) {
                Some(Node::Dir(_)) => FileType::Directory,
                _ => FileType::RegularFile,
            };
            entries.push((*cino, kind, name.clone()));
        }
        for (i, (eino, kind, name)) in entries.into_iter().enumerate().skip(offset.max(0) as usize)
        {
            if reply.add(eino, (i + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: emlfs <store-dir> <mountpoint>");
        std::process::exit(2);
    }
    let store = Path::new(&args[1]);
    let mount = Path::new(&args[2]);
    let tree = match Tree::build_store(store) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("emlfs: {}", e);
            std::process::exit(1);
        }
    };
    eprintln!(
        "emlfs: {} inodes from {} -> {}",
        tree.inode_count(),
        store.display(),
        mount.display()
    );
    let fs = EmlFs { tree };
    let opts = [MountOption::RO, MountOption::FSName("emlfs".to_string())];
    if let Err(e) = fuser::mount2(fs, mount, &opts) {
        eprintln!("emlfs: mount failed: {}", e);
        std::process::exit(1);
    }
}
