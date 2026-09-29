//! emlui — interactive ANSI dashboard for the eml-linux stack.
//!
//!   emlui [--root DIR] [--once] [--interval SECS] [--recent N]
//!
//! Interactive keys: q quit, r refresh, j/k or arrows move the selection,
//! enter/p to pin the selected spool as the detail panel, g/G first/last.
//! Falls back to a one-shot render when stdout is not a TTY.

use emlcore::Spool;
use emlctl::{scan_spools, SpoolInfo};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const REV: &str = "\x1b[7m";
const HOME: &str = "\x1b[H";

/// RAII raw-mode + alternate-screen guard.
struct Raw {
    saved: libc::termios,
}

impl Raw {
    fn enable() -> Option<Raw> {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return None;
            }
            let saved = t;
            t.c_lflag &= !(libc::ICANON | libc::ECHO);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 1; // 100 ms read timeout
            if libc::tcsetattr(0, libc::TCSANOW, &t) != 0 {
                return None;
            }
            print!("\x1b[?1049h\x1b[?25l"); // alt screen, hide cursor
            let _ = std::io::stdout().flush();
            Some(Raw { saved })
        }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.saved);
        }
        print!("\x1b[?1049l\x1b[?25h");
        let _ = std::io::stdout().flush();
    }
}

fn read_key() -> Option<u8> {
    let mut b = [0u8; 1];
    match std::io::stdin().read(&mut b) {
        Ok(1) => Some(b[0]),
        _ => None,
    }
}

fn compact(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 64 {
        format!("{}…", &s[..63])
    } else {
        s
    }
}

/// One full frame (pure over the spool list, so it is testable).
pub fn render_frame(spools: &[SpoolInfo], root: &str, selected: usize, recent: usize) -> String {
    let mut out = String::new();
    out.push_str(HOME);
    out.push_str(&format!(
        "{BOLD}{CYAN}eml-linux{RESET}  root={}  spools={}  {DIM}[q]uit [r]efresh [j/k] move{RESET}\n",
        root,
        spools.len()
    ));
    out.push_str(&format!("{DIM}{}{RESET}\n", "-".repeat(76)));

    let mut total = 0usize;
    let mut broken = 0usize;
    for (i, s) in spools.iter().enumerate() {
        total += s.events;
        if !s.chain_ok {
            broken += 1;
        }
        let mark = if i == selected { format!("{REV}>{RESET}") } else { " ".into() };
        let status = if s.chain_ok {
            format!("{GREEN}ok{RESET}")
        } else {
            format!("{RED}BROKEN{RESET}")
        };
        out.push_str(&format!(
            "{} {:<40} {:>5} ev  {}\n",
            mark,
            s.dir.display().to_string(),
            s.events,
            status
        ));
    }
    if spools.is_empty() {
        out.push_str("  (no spools found)\n");
    }

    out.push_str(&format!("{DIM}{}{RESET}\n", "-".repeat(76)));
    if let Some(s) = spools.get(selected) {
        out.push_str(&format!(
            "{BOLD}detail{RESET}: {}\n",
            s.dir.display()
        ));
        if let Ok(sp) = Spool::open(&s.dir) {
            let start = sp.len().saturating_sub(recent);
            for ev in &sp.events()[start..] {
                out.push_str(&format!(
                    "  {DIM}#{:<3}{RESET} {:<12} {:<9} {}\n",
                    ev.seq,
                    ev.kind,
                    ev.source,
                    compact(&ev.payload)
                ));
            }
        }
    }
    out.push_str(&format!("{DIM}{}{RESET}\n", "-".repeat(76)));
    out.push_str(&format!(
        "{BOLD}total{RESET}: {} events / {} spool(s), {} broken   head {}\n",
        total,
        spools.len(),
        broken,
        spools.first().map(|s| s.head.as_str()).unwrap_or("-")
    ));
    out
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

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn stdout_is_tty() -> bool {
    unsafe { libc::isatty(1) == 1 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = flag(&args, "--root").unwrap_or_else(|| "run".into());
    let recent: usize = flag(&args, "--recent").and_then(|s| s.parse().ok()).unwrap_or(4);
    let interval: u64 = flag(&args, "--interval").and_then(|s| s.parse().ok()).unwrap_or(2);

    let root_path = PathBuf::from(&root);

    if has(&args, "--once") || !stdout_is_tty() {
        let spools = scan_spools(&root_path, 3);
        print!("{}", render_frame(&spools, &root, 0, recent));
        return;
    }

    let _raw = match Raw::enable() {
        Some(r) => r,
        None => {
            let spools = scan_spools(&root_path, 3);
            print!("{}", render_frame(&spools, &root, 0, recent));
            return;
        }
    };

    let mut spools: Vec<SpoolInfo> = scan_spools(&root_path, 3);
    let mut selected = 0usize;
    let mut last = std::time::Instant::now();

    loop {
        print!("{}", render_frame(&spools, &root, selected, recent));
        let _ = std::io::stdout().flush();

        if let Some(k) = read_key() {
            match k {
                b'q' | 3 => break,                    // q or Ctrl-C
                b'r' => {
                    spools = scan_spools(&root_path, 3);
                    if selected >= spools.len() {
                        selected = spools.len().saturating_sub(1);
                    }
                    last = std::time::Instant::now();
                }
                b'j' | b'\x1b' => {
                    // ESC alone or ESC [ B treated as down for simplicity
                    if !spools.is_empty() {
                        selected = (selected + 1).min(spools.len() - 1);
                    }
                }
                b'k' => {
                    selected = selected.saturating_sub(1);
                }
                b'g' => selected = 0,
                b'G' => selected = spools.len().saturating_sub(1),
                b'B' => {
                    // read the rest of a CSI arrow sequence if present
                    for _ in 0..2 {
                        let _ = read_key();
                    }
                    if !spools.is_empty() {
                        selected = (selected + 1).min(spools.len() - 1);
                    }
                }
                b'A' => {
                    for _ in 0..2 {
                        let _ = read_key();
                    }
                    selected = selected.saturating_sub(1);
                }
                _ => {}
            }
        } else if last.elapsed() >= Duration::from_secs(interval) {
            spools = scan_spools(&root_path, 3);
            last = std::time::Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spool(name: &str, events: usize, ok: bool) -> SpoolInfo {
        SpoolInfo {
            dir: PathBuf::from(name),
            events,
            chain_ok: ok,
            head: "abc123".into(),
        }
    }

    #[test]
    fn frame_lists_spools_and_totals() {
        let s = vec![spool("/tmp/a", 3, true), spool("/tmp/b", 5, false)];
        let f = render_frame(&s, "/tmp", 1, 4);
        assert!(f.contains("/tmp/a"));
        assert!(f.contains("/tmp/b"));
        assert!(f.contains("BROKEN"));
        assert!(f.contains("8 events / 2 spool(s), 1 broken"));
    }

    #[test]
    fn frame_empty() {
        let f = render_frame(&[], "/x", 0, 4);
        assert!(f.contains("no spools found"));
    }
}
