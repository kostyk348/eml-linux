//! emlui — a dependency-free ANSI dashboard for the eml-linux stack.
//!
//!   emlui [--root DIR] [--once] [--interval SECS] [--count N]
//!
//! Redraws a screen with every spool found under `root`, its event count,
//! hash-chain status and the most recent events.

use emlcore::Spool;
use emlctl::scan_spools;
use std::path::PathBuf;
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const CLEAR: &str = "\x1b[2J\x1b[H";

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

/// Build one dashboard frame (pure function, testable).
pub fn render(root: &str, recent: usize) -> String {
    let spools = scan_spools(&PathBuf::from(root), 3);
    let mut out = String::new();
    out.push_str(&format!(
        "{BOLD}{CYAN}eml-linux dashboard{RESET}  root={}  spools={}\n",
        root,
        spools.len()
    ));
    out.push_str(&format!("{DIM}{}{RESET}\n", "-".repeat(72)));

    let mut total_events = 0usize;
    let mut broken = 0usize;
    for s in &spools {
        total_events += s.events;
        if !s.chain_ok {
            broken += 1;
        }
        let status = if s.chain_ok {
            format!("{GREEN}ok{RESET}")
        } else {
            format!("{RED}BROKEN{RESET}")
        };
        out.push_str(&format!(
            "  {:<38} {:>5} events  chain {}\n",
            s.dir.display().to_string(),
            s.events,
            status
        ));
        if let Ok(sp) = Spool::open(&s.dir) {
            let start = sp.len().saturating_sub(recent);
            for ev in &sp.events()[start..] {
                out.push_str(&format!(
                    "      {DIM}#{:<4}{RESET} {:<12} {:<10} {}\n",
                    ev.seq,
                    ev.kind,
                    ev.source,
                    serde_json_compact(&ev.payload)
                ));
            }
        }
    }
    if spools.is_empty() {
        out.push_str("  (no spools found)\n");
    }
    out.push_str(&format!("{DIM}{}{RESET}\n", "-".repeat(72)));
    out.push_str(&format!(
        "{BOLD}total{RESET}: {} events across {} spool(s); {} broken\n",
        total_events,
        spools.len(),
        broken
    ));
    out
}

/// Minimal JSON compaction without pulling serde_json into the UI crate.
fn serde_json_compact(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 60 {
        format!("{}…", &s[..59])
    } else {
        s
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = flag(&args, "--root").unwrap_or_else(|| "run".into());
    let once = has(&args, "--once");
    let interval: u64 = flag(&args, "--interval").and_then(|s| s.parse().ok()).unwrap_or(2);
    let count: Option<u64> = flag(&args, "--count").and_then(|s| s.parse().ok());
    let recent: usize = flag(&args, "--recent").and_then(|s| s.parse().ok()).unwrap_or(3);

    if once {
        print!("{CLEAR}{}", render(&root, recent));
        return;
    }
    let mut frames = 0u64;
    loop {
        print!("{CLEAR}{}", render(&root, recent));
        use std::io::Write;
        let _ = std::io::stdout().flush();
        frames += 1;
        if let Some(c) = count {
            if frames >= c {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_empty_root() {
        let s = render("/nonexistent-emlui-root", 3);
        assert!(s.contains("dashboard"));
        assert!(s.contains("no spools found"));
    }
}
