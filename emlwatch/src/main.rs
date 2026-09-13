//! emlwatch CLI — stream inotify events as `.eml`.
//!
//!   emlwatch --dir DIR --spool DIR [--mask create,modify,delete] [--max-events N]

use emlcore::Spool;
use emlwatch::{describe, event_kind, parse_watch_mask};
use inotify::Inotify;
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
    let dir = PathBuf::from(flag(&args, "--dir").unwrap_or_else(|| ".".into()));
    let spool_dir = flag(&args, "--spool").unwrap_or_else(|| "run/emlwatch".into());
    let mask_spec = flag(&args, "--mask").unwrap_or_else(|| "all".into());
    let max_events: Option<u64> = flag(&args, "--max-events").and_then(|s| s.parse().ok());

    let mut spool = match Spool::open(&spool_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlwatch: {}", e);
            std::process::exit(1);
        }
    };
    let mut inotify = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("emlwatch: inotify init: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = inotify
        .watches()
        .add(&dir, parse_watch_mask(&mask_spec))
    {
        eprintln!("emlwatch: watch {}: {}", dir.display(), e);
        std::process::exit(1);
    }
    let _ = spool.append(
        "WATCH_START",
        "emlwatch",
        serde_json::json!({"dir": dir.to_string_lossy(), "mask": mask_spec}),
    );
    println!("emlwatch: watching {} -> {}", dir.display(), spool_dir);

    let mut buffer = [0u8; 4096];
    let mut count = 0u64;
    loop {
        let events = match inotify.read_events_blocking(&mut buffer) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("emlwatch: {}", e);
                break;
            }
        };
        for event in events {
            let name = event
                .name
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let kind = event_kind(event.mask);
            let _ = spool.append(
                kind,
                "emlwatch",
                serde_json::json!({
                    "path": name, "wd": format!("{:?}", event.wd),
                    "flags": describe(event.mask), "cookie": event.cookie
                }),
            );
            println!("{} {}", kind, name);
            count += 1;
            if let Some(m) = max_events {
                if count >= m {
                    let _ = spool.append("WATCH_STOP", "emlwatch", serde_json::json!({"events": count}));
                    println!("emlwatch: done ({} events)", count);
                    return;
                }
            }
        }
    }
    let st = spool.verify();
    println!("emlwatch: done ({} events, chain {})", count, if st.ok { "ok" } else { "BROKEN" });
}
