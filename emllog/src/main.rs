//! emllog — journald replacement: tail / verify / grep over a hash-chained .eml spool.
//!
//!   emllog [--spool DIR] tail [N]
//!   emllog [--spool DIR] verify
//!   emllog [--spool DIR] grep PATTERN
//!   emllog [--spool DIR] stats

use emlcore::Spool;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut spool_dir = PathBuf::from("run/emlinit");
    let mut rest: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--spool" {
            i += 1;
            if let Some(v) = args.get(i) {
                spool_dir = PathBuf::from(v);
            }
        } else {
            rest.push(args[i].clone());
        }
        i += 1;
    }
    let cmd = rest.first().map(|s| s.as_str()).unwrap_or("tail");
    let spool = match Spool::open(&spool_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emllog: {}", e);
            std::process::exit(1);
        }
    };
    let code = match cmd {
        "tail" => {
            let n: usize = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
            let start = spool.len().saturating_sub(n);
            for ev in &spool.events()[start..] {
                println!(
                    "{:>5} {:<9} {:<9} {:<24} {}",
                    ev.seq,
                    ev.kind,
                    ev.source,
                    ev.ts,
                    serde_json::to_string(&ev.payload).unwrap_or_default()
                );
            }
            0
        }
        "grep" => {
            let pat = rest.get(1).cloned().unwrap_or_default();
            let needle = pat.to_lowercase();
            let mut hits = 0;
            for ev in spool.events() {
                let line = format!(
                    "{} {} {} {}",
                    ev.kind,
                    ev.source,
                    ev.ts,
                    serde_json::to_string(&ev.payload).unwrap_or_default()
                );
                if line.to_lowercase().contains(&needle) {
                    hits += 1;
                    println!("{:>5} {}", ev.seq, line);
                }
            }
            println!("-- {} match(es)", hits);
            0
        }
        "verify" => {
            let st = spool.verify();
            if st.ok {
                println!("OK: {} events, chain intact (head {})", st.checked, spool.head());
                0
            } else {
                println!("BROKEN at seq {:?} after {} events", st.broken_at, st.checked);
                1
            }
        }
        "stats" => {
            use std::collections::BTreeMap;
            let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
            for ev in spool.events() {
                *by_kind.entry(ev.kind.as_str()).or_insert(0) += 1;
            }
            println!("events: {}", spool.len());
            println!("head:   {}", spool.head());
            for (k, n) in by_kind {
                println!("  {:<10} {}", k, n);
            }
            0
        }
        _ => {
            eprintln!("usage: emllog [--spool DIR] tail [N] | verify | grep PAT | stats");
            2
        }
    };
    std::process::exit(code);
}
