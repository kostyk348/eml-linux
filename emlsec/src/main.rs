//! emlsec CLI — encrypted secrets as .eml.
//!
//!   emlsec init --store DIR
//!   emlsec set  --store DIR --name N --value V
//!   emlsec get  --store DIR --name N
//!   emlsec list --store DIR
//!   emlsec rm   --store DIR --name N
//!   emlsec log  --store DIR

use emlcore::{Record, Spool};
use emlsec::{from_hex, open_record, secret_record, to_hex};
use rand::RngCore;
use std::fs;
use std::path::{Path, PathBuf};

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

fn store_dir(args: &[String]) -> PathBuf {
    PathBuf::from(flag(args, "--store").unwrap_or_else(|| "run/emlsec".into()))
}

fn load_key(store: &Path) -> Result<[u8; 32], String> {
    if let Ok(env) = std::env::var("EMLSEC_KEY") {
        let b = from_hex(&env)?;
        if b.len() == 32 {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            return Ok(a);
        }
        return Err("EMLSEC_KEY must be 32 hex bytes".into());
    }
    let kp = store.join("key");
    let hex = fs::read_to_string(&kp).map_err(|_| format!("no key at {} (run `emlsec init`)", kp.display()))?;
    let b = from_hex(hex.trim())?;
    if b.len() != 32 {
        return Err("key must be 32 bytes".into());
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&b);
    Ok(a)
}

fn secret_path(store: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    store.join(format!("{}.secret.eml", safe))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("list");
    let store = store_dir(&args);
    let _ = fs::create_dir_all(&store);
    let code = match cmd {
        "init" => {
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            let kp = store.join("key");
            if let Err(e) = fs::write(&kp, to_hex(&key) + "\n") {
                eprintln!("emlsec: {}", e);
                std::process::exit(1);
            }
            let _ = fs::set_permissions(&kp, std::os::unix::fs::PermissionsExt::from_mode(0o600));
            println!("key written to {}", kp.display());
            0
        }
        "set" => {
            let key = match load_key(&store) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("emlsec: {}", e);
                    std::process::exit(1);
                }
            };
            let name = flag(&args, "--name").unwrap_or_default();
            let value = flag(&args, "--value").unwrap_or_default();
            let rec = match secret_record(&name, &key, value.as_bytes()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("emlsec: {}", e);
                    std::process::exit(1);
                }
            };
            let p = secret_path(&store, &name);
            if let Err(e) = fs::write(&p, rec.render()) {
                eprintln!("emlsec: {}", e);
                std::process::exit(1);
            }
            if let Ok(mut log) = Spool::open(store.join("access")) {
                let _ = log.append("SET", "emlsec", serde_json::json!({"name": name}));
            }
            println!("stored {}", name);
            0
        }
        "get" => {
            let key = match load_key(&store) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("emlsec: {}", e);
                    std::process::exit(1);
                }
            };
            let name = flag(&args, "--name").unwrap_or_default();
            let p = secret_path(&store, &name);
            let data = match fs::read(&p) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("emlsec: {}: {}", p.display(), e);
                    std::process::exit(1);
                }
            };
            let rec = Record::parse(&data);
            match open_record(&rec, &key) {
                Ok(pt) => {
                    println!("{}", String::from_utf8_lossy(&pt));
                    if let Ok(mut log) = Spool::open(store.join("access")) {
                        let _ = log.append("GET", "emlsec", serde_json::json!({"name": name}));
                    }
                    0
                }
                Err(e) => {
                    eprintln!("emlsec: decrypt: {}", e);
                    1
                }
            }
        }
        "list" => {
            let mut names = Vec::new();
            for e in fs::read_dir(&store).into_iter().flatten().flatten() {
                let p = e.path();
                if p.to_string_lossy().ends_with(".secret.eml") {
                    if let Ok(d) = fs::read(&p) {
                        let rec = Record::parse(&d);
                        names.push(rec.get("X-Entity-ID").unwrap_or("").to_string());
                    }
                }
            }
            names.sort();
            for n in names {
                println!("{}", n);
            }
            0
        }
        "rm" => {
            let name = flag(&args, "--name").unwrap_or_default();
            let p = secret_path(&store, &name);
            match fs::remove_file(&p) {
                Ok(()) => {
                    if let Ok(mut log) = Spool::open(store.join("access")) {
                        let _ = log.append("DEL", "emlsec", serde_json::json!({"name": name}));
                    }
                    println!("removed {}", name);
                    0
                }
                Err(e) => {
                    eprintln!("emlsec: {}", e);
                    1
                }
            }
        }
        "log" => {
            let spool = match Spool::open(store.join("access")) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlsec: {}", e);
                    std::process::exit(1);
                }
            };
            for ev in spool.events() {
                println!("{:<5} {:<9} {}", ev.seq, ev.kind, ev.payload);
            }
            let st = spool.verify();
            println!("chain: {}", if st.ok { "ok" } else { "BROKEN" });
            if st.ok { 0 } else { 1 }
        }
        _ => {
            eprintln!("usage: emlsec init|set|get|list|rm|log --store DIR ...");
            2
        }
    };
    std::process::exit(code);
}
