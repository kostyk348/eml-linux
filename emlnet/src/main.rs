//! emlnet CLI — replicated CRDT store over TCP.
//!
//!   emlnet put   --store DIR --writer W --key K --value JSON
//!   emlnet get   --store DIR [--key K]
//!   emlnet serve --store DIR --addr 127.0.0.1:PORT [--once]
//!   emlnet sync  --store DIR --peer 127.0.0.1:PORT

use emlnet::Store;
use std::net::TcpListener;

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

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("get");
    let store_dir = flag(&args, "--store").unwrap_or_else(|| "run/emlnet".into());
    let code = match cmd {
        "put" => {
            let mut s = match Store::open(&store_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlnet: {}", e);
                    std::process::exit(1);
                }
            };
            let writer = flag(&args, "--writer").unwrap_or_else(|| "local".into());
            let key = flag(&args, "--key").unwrap_or_default();
            let value: serde_json::Value = flag(&args, "--value")
                .and_then(|v| serde_json::from_str(&v).ok())
                .unwrap_or(serde_json::Value::Null);
            match s.put(&writer, &key, value) {
                Ok(op) => {
                    println!("op {} key={} ts={}", op.id, op.key, op.ts);
                    0
                }
                Err(e) => {
                    eprintln!("emlnet: {}", e);
                    1
                }
            }
        }
        "get" => {
            let s = match Store::open(&store_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlnet: {}", e);
                    std::process::exit(1);
                }
            };
            let state = s.state();
            match flag(&args, "--key") {
                Some(k) => println!("{}", serde_json::to_string_pretty(&state.get(&k)).unwrap()),
                None => println!("{}", serde_json::to_string_pretty(&state).unwrap()),
            }
            0
        }
        "serve" => {
            let mut s = match Store::open(&store_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlnet: {}", e);
                    std::process::exit(1);
                }
            };
            let addr = flag(&args, "--addr").unwrap_or_else(|| "127.0.0.1:9099".into());
            let listener = match TcpListener::bind(&addr) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("emlnet: bind {}: {}", addr, e);
                    std::process::exit(1);
                }
            };
            println!("emlnet: serving {} ({} ops)", addr, s.len());
            loop {
                if let Err(e) = s.serve_once(&listener) {
                    eprintln!("emlnet: {}", e);
                } else {
                    println!("emlnet: synced ({} ops)", s.len());
                }
                if has(&args, "--once") {
                    break;
                }
            }
            0
        }
        "sync" => {
            let mut s = match Store::open(&store_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlnet: {}", e);
                    std::process::exit(1);
                }
            };
            let peer = flag(&args, "--peer").unwrap_or_else(|| "127.0.0.1:9099".into());
            match s.sync_to(&peer) {
                Ok(n) => {
                    println!("emlnet: merged {} new op(s), {} total", n, s.len());
                    0
                }
                Err(e) => {
                    eprintln!("emlnet: sync {}: {}", peer, e);
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: emlnet put|get|serve|sync --store DIR ...");
            2
        }
    };
    std::process::exit(code);
}
