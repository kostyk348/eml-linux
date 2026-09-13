//! emlpkg — a package IS a single `.eml` container.
//!
//!   emlpkg build   <dir> <out.eml> [--id ID] [--version V]
//!   emlpkg info    <pkg.eml>
//!   emlpkg verify  <pkg.eml>
//!   emlpkg install <pkg.eml> --root DIR --log DIR
//!   emlpkg rollback --root DIR --log DIR
//!
//! Body layout (binary-safe): a single-line JSON manifest, `\n`, then the raw
//! concatenated file blob. The manifest carries (path, mode, sha256, off, len)
//! for every entry, so verification is a slice + hash.

use emlcore::{Record, Spool};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct FileEnt {
    path: String,
    mode: String,
    sha256: String,
    off: u64,
    len: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Manifest {
    id: String,
    version: String,
    arch: String,
    depends: Vec<String>,
    blob_len: u64,
    files: Vec<FileEnt>,
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("info");
    let code = match cmd {
        "build" => cmd_build(&args[2..]),
        "info" => cmd_info(&args[2..]),
        "verify" => cmd_verify(&args[2..]),
        "install" => cmd_install(&args[2..]),
        "rollback" => cmd_rollback(&args[2..]),
        _ => {
            eprintln!("usage: emlpkg build|info|verify|install|rollback ...");
            2
        }
    };
    std::process::exit(code);
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf, u32)>) -> std::io::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name == ".emlpkg" {
            continue;
        }
        let md = e.metadata()?;
        if md.is_dir() {
            walk(&p, base, out)?;
        } else if md.is_file() {
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            out.push((rel, p.clone(), md.permissions().mode() & 0o777));
        }
    }
    Ok(())
}

fn pack_dir(dir: &Path, id: &str, version: &str) -> Result<(Manifest, Vec<u8>), String> {
    let mut entries = Vec::new();
    walk(dir, dir, &mut entries).map_err(|e| e.to_string())?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut blob: Vec<u8> = Vec::new();
    let mut files = Vec::new();
    for (rel, p, mode) in entries {
        let data = fs::read(&p).map_err(|e| e.to_string())?;
        let off = blob.len() as u64;
        let len = data.len() as u64;
        let digest = sha256_hex(&data);
        blob.extend_from_slice(&data);
        files.push(FileEnt {
            path: rel,
            mode: format!("{:04o}", mode),
            sha256: digest,
            off,
            len,
        });
    }
    let manifest = Manifest {
        id: id.to_string(),
        version: version.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        depends: Vec::new(),
        blob_len: blob.len() as u64,
        files,
    };
    Ok((manifest, blob))
}

fn render_pkg(manifest: &Manifest, blob: &[u8]) -> Vec<u8> {
    let mut rec = Record::new();
    rec.set("From", format!("<{}@eml.local>", manifest.id));
    rec.set("To", "<host@eml.local>");
    rec.set("Subject", format!("{} {}", manifest.id, manifest.version));
    rec.set("X-EML-Type", "Application/Package");
    rec.set("X-Entity-ID", manifest.id.clone());
    rec.set("X-Pkg-Version", manifest.version.clone());
    rec.set("X-Pkg-Arch", manifest.arch.clone());
    rec.set("Content-Type", "application/x-emlpkg; charset=utf-8");
    let json = serde_json::to_string(manifest).unwrap_or_default();
    let mut body = json.into_bytes();
    body.push(b'\n');
    body.extend_from_slice(blob);
    rec.body = body;
    rec.render()
}

fn parse_pkg(path: &Path) -> Result<(Record, Manifest, Vec<u8>), String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let rec = Record::parse(&data);
    let body = &rec.body;
    let nl = body
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| "package body: missing manifest line".to_string())?;
    let manifest: Manifest =
        serde_json::from_slice(&body[..nl]).map_err(|e| format!("manifest: {}", e))?;
    let blob = body[nl + 1..].to_vec();
    Ok((rec, manifest, blob))
}

fn cmd_build(args: &[String]) -> i32 {
    let dir = match args.first() {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("usage: emlpkg build <dir> <out.eml> [--id ID] [--version V]");
            return 2;
        }
    };
    let out = match args.get(1) {
        Some(o) => PathBuf::from(o),
        None => {
            eprintln!("emlpkg: missing <out.eml>");
            return 2;
        }
    };
    let id = flag(args, "--id").unwrap_or_else(|| {
        dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "pkg".into())
    });
    let version = flag(args, "--version").unwrap_or_else(|| "0.0.0".into());
    match pack_dir(&dir, &id, &version) {
        Ok((m, blob)) => {
            let bytes = render_pkg(&m, &blob);
            if let Err(e) = fs::write(&out, bytes) {
                eprintln!("emlpkg: write {}: {}", out.display(), e);
                return 1;
            }
            println!(
                "built {} v{}: {} file(s), {} bytes -> {}",
                m.id,
                m.version,
                m.files.len(),
                m.blob_len,
                out.display()
            );
            0
        }
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            1
        }
    }
}

fn cmd_info(args: &[String]) -> i32 {
    let path = match args.first() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: emlpkg info <pkg.eml>");
            return 2;
        }
    };
    match parse_pkg(&path) {
        Ok((_, m, blob)) => {
            println!("id:      {}", m.id);
            println!("version: {}", m.version);
            println!("arch:    {}", m.arch);
            println!("blob:    {} bytes", blob.len());
            println!("files:   {}", m.files.len());
            for f in &m.files {
                println!("  {:04o} {:>8}  {}  {}", u32::from_str_radix(&f.mode, 8).unwrap_or(0), f.len, &f.sha256[..12], f.path);
            }
            0
        }
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            1
        }
    }
}

fn cmd_verify(args: &[String]) -> i32 {
    let path = match args.first() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: emlpkg verify <pkg.eml>");
            return 2;
        }
    };
    match parse_pkg(&path) {
        Ok((_, m, blob)) => {
            if blob.len() as u64 != m.blob_len {
                println!("BROKEN: blob length {} != manifest {}", blob.len(), m.blob_len);
                return 1;
            }
            let mut bad = 0;
            for f in &m.files {
                let (s, e) = (f.off as usize, (f.off + f.len) as usize);
                if e > blob.len() {
                    println!("BROKEN: {} out of range", f.path);
                    bad += 1;
                    continue;
                }
                let got = sha256_hex(&blob[s..e]);
                if got != f.sha256 {
                    println!("BROKEN: {} hash mismatch", f.path);
                    bad += 1;
                }
            }
            if bad == 0 {
                println!("OK: {} file(s) verified, blob {} bytes", m.files.len(), blob.len());
                0
            } else {
                println!("{} file(s) FAILED", bad);
                1
            }
        }
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            1
        }
    }
}

fn cmd_install(args: &[String]) -> i32 {
    let path = match args.first() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: emlpkg install <pkg.eml> --root DIR --log DIR");
            return 2;
        }
    };
    let root = match flag(args, "--root") {
        Some(r) => PathBuf::from(r),
        None => {
            eprintln!("emlpkg: --root required");
            return 2;
        }
    };
    let log_dir = flag(args, "--log").unwrap_or_else(|| "emlpkg-log".into());
    let (_, m, blob) = match parse_pkg(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            return 1;
        }
    };

    // verify before touching the filesystem
    for f in &m.files {
        let (s, e) = (f.off as usize, (f.off + f.len) as usize);
        if e > blob.len() || sha256_hex(&blob[s..e]) != f.sha256 {
            eprintln!("emlpkg: refusing to install — {} failed verification", f.path);
            return 1;
        }
    }

    let mut spool = match Spool::open(&log_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            return 1;
        }
    };
    let seq = spool.len() + 1;
    let backup = root.join(".emlpkg").join("backup").join(seq.to_string());
    fs::create_dir_all(&backup).ok();

    let mut records = Vec::new();
    for f in &m.files {
        let dest = root.join(&f.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ok();
        }
        let existed = dest.exists();
        if existed {
            let bpath = backup.join(&f.path);
            if let Some(parent) = bpath.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::copy(&dest, &bpath).ok();
        }
        let (s, e) = (f.off as usize, (f.off + f.len) as usize);
        let data = &blob[s..e];
        let tmp = dest.with_extension("emlpkg.tmp");
        if let Err(err) = fs::write(&tmp, data) {
            eprintln!("emlpkg: write {}: {}", dest.display(), err);
            return 1;
        }
        if let Ok(mode) = u32::from_str_radix(&f.mode, 8) {
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode));
        }
        if let Err(err) = fs::rename(&tmp, &dest) {
            eprintln!("emlpkg: rename {}: {}", dest.display(), err);
            return 1;
        }
        records.push(serde_json::json!({
            "path": f.path, "mode": f.mode, "sha256": f.sha256, "existed_before": existed
        }));
    }

    let ev = spool.append(
        "INSTALL",
        "emlpkg",
        serde_json::json!({
            "id": m.id, "version": m.version, "root": root.to_string_lossy(),
            "backup": backup.to_string_lossy(), "files": records
        }),
    );
    match ev {
        Ok(e) => {
            println!("installed {} v{}: {} file(s), event seq {}", m.id, m.version, m.files.len(), e.seq);
            0
        }
        Err(e) => {
            eprintln!("emlpkg: log: {}", e);
            1
        }
    }
}

fn cmd_rollback(args: &[String]) -> i32 {
    let root = match flag(args, "--root") {
        Some(r) => PathBuf::from(r),
        None => {
            eprintln!("emlpkg: --root required");
            return 2;
        }
    };
    let log_dir = flag(args, "--log").unwrap_or_else(|| "emlpkg-log".into());
    let spool = match Spool::open(&log_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlpkg: {}", e);
            return 1;
        }
    };
    let last = spool
        .events()
        .iter()
        .rev()
        .find(|e| e.kind == "INSTALL");
    let last = match last {
        Some(e) => e.clone(),
        None => {
            eprintln!("emlpkg: no INSTALL event in {}", log_dir);
            return 1;
        }
    };
    let backup = last
        .payload
        .get("backup")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    let files = last
        .payload
        .get("files")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut restored = 0;
    let mut removed = 0;
    for f in &files {
        let rel = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let existed = f
            .get("existed_before")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dest = root.join(rel);
        if existed {
            if let Some(b) = &backup {
                let bpath = b.join(rel);
                if bpath.exists() {
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    fs::copy(&bpath, &dest).ok();
                    restored += 1;
                }
            }
        } else if dest.exists() {
            fs::remove_file(&dest).ok();
            removed += 1;
        }
    }

    let mut spool = spool;
    let ev = spool.append(
        "ROLLBACK",
        "emlpkg",
        serde_json::json!({
            "reverted_install_seq": last.seq,
            "id": last.payload.get("id"),
            "restored": restored, "removed": removed
        }),
    );
    match ev {
        Ok(e) => {
            println!("rolled back install seq {}: {} restored, {} removed (event seq {})", last.seq, restored, removed, e.seq);
            0
        }
        Err(e) => {
            eprintln!("emlpkg: log: {}", e);
            1
        }
    }
}
