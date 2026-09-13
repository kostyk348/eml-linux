//! emlsnap CLI — snapshot / verify / restore a .eml store.
//!
//!   emlsnap create  --src DIR --out DIR
//!   emlsnap verify  --snap DIR
//!   emlsnap restore --snap DIR --out DIR

use emlsnap::{restore, snapshot, verify};
use std::path::PathBuf;

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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("verify");
    let code = match cmd {
        "create" => {
            let src = PathBuf::from(flag(&args, "--src").unwrap_or_else(|| ".".into()));
            let out = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| "snap".into()));
            match snapshot(&src, &out) {
                Ok(n) => {
                    println!("snapshot: {} file(s) -> {}", n, out.display());
                    0
                }
                Err(e) => {
                    eprintln!("emlsnap: {}", e);
                    1
                }
            }
        }
        "verify" => {
            let snap = PathBuf::from(flag(&args, "--snap").unwrap_or_else(|| "snap".into()));
            match verify(&snap) {
                Ok(problems) if problems.is_empty() => {
                    println!("OK: snapshot intact");
                    0
                }
                Ok(problems) => {
                    for p in &problems {
                        println!("BROKEN: {}", p);
                    }
                    1
                }
                Err(e) => {
                    eprintln!("emlsnap: {}", e);
                    1
                }
            }
        }
        "restore" => {
            let snap = PathBuf::from(flag(&args, "--snap").unwrap_or_else(|| "snap".into()));
            let out = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| "restored".into()));
            match restore(&snap, &out) {
                Ok(n) => {
                    println!("restored {} file(s) -> {}", n, out.display());
                    0
                }
                Err(e) => {
                    eprintln!("emlsnap: {}", e);
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: emlsnap create|verify|restore --src/--snap/--out DIR");
            2
        }
    };
    std::process::exit(code);
}
