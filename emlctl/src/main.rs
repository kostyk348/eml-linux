//! emlctl CLI — unified control plane for the eml-linux stack.
//!
//!   emlctl doctor
//!   emlctl status --root DIR
//!   emlctl verify --root DIR
//!   emlctl up     --root DIR [--max-runtime SECS]

use emlctl::scan_spools;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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

fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn exists(p: &Path) -> bool {
    p.exists()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("status");
    let root = PathBuf::from(flag(&args, "--root").unwrap_or_else(|| "run".into()));
    let code = match cmd {
        "doctor" => {
            println!("emlctl doctor");
            let checks = [
                ("/dev/fuse", exists(Path::new("/dev/fuse"))),
                ("fusermount3", exists(Path::new("/usr/bin/fusermount3"))),
                ("dinit", exists(Path::new("/usr/bin/dinit"))),
                ("emlinit", exists(&sibling("emlinit"))),
                ("emlcron", exists(&sibling("emlcron"))),
                ("emlwatch", exists(&sibling("emlwatch"))),
                ("emlsign", exists(&sibling("emlsign"))),
                ("emlnet", exists(&sibling("emlnet"))),
            ];
            let mut ok = true;
            for (name, present) in checks {
                println!("  [{}] {}", if present { "ok" } else { "--" }, name);
                if !present {
                    ok = false;
                }
            }
            if ok {
                0
            } else {
                1
            }
        }
        "status" => {
            let spools = scan_spools(&root, 3);
            if spools.is_empty() {
                println!("no spools under {}", root.display());
            }
            for s in spools {
                println!(
                    "{:<40} {:>5} events  chain {}  head {}",
                    s.dir.display(),
                    s.events,
                    if s.chain_ok { "ok" } else { "BROKEN" },
                    s.head
                );
            }
            0
        }
        "verify" => {
            let spools = scan_spools(&root, 3);
            let bad: Vec<_> = spools.iter().filter(|s| !s.chain_ok).collect();
            println!("{} spool(s), {} broken", spools.len(), bad.len());
            for s in &bad {
                println!("BROKEN: {}", s.dir.display());
            }
            if bad.is_empty() {
                0
            } else {
                1
            }
        }
        "up" => {
            let max_runtime: Option<u64> =
                flag(&args, "--max-runtime").and_then(|s| s.parse().ok());
            let run = root.join("run");
            let _ = std::fs::create_dir_all(&run);
            let mut children: Vec<Child> = Vec::new();

            let services = root.join("services");
            if services.is_dir() {
                let bin = sibling("emlinit");
                println!("emlctl: starting emlinit on {}", services.display());
                children.push(
                    Command::new(bin)
                        .args(["run", "--units"])
                        .arg(&services)
                        .args(["--spool"])
                        .arg(run.join("emlinit"))
                        .stdin(Stdio::null())
                        .spawn()
                        .expect("spawn emlinit"),
                );
            }
            let jobs = root.join("jobs");
            if jobs.is_dir() {
                println!("emlctl: starting emlcron on {}", jobs.display());
                children.push(
                    Command::new(sibling("emlcron"))
                        .args(["run", "--jobs"])
                        .arg(&jobs)
                        .args(["--spool"])
                        .arg(run.join("emlcron"))
                        .stdin(Stdio::null())
                        .spawn()
                        .expect("spawn emlcron"),
                );
            }
            let watches = root.join("watches");
            if watches.is_dir() {
                for e in std::fs::read_dir(&watches).into_iter().flatten().flatten() {
                    let wd = e.path();
                    if wd.is_dir() {
                        let name = wd.file_name().unwrap().to_string_lossy().to_string();
                        println!("emlctl: watching {} -> {}", wd.display(), name);
                        children.push(
                            Command::new(sibling("emlwatch"))
                                .args(["--dir"])
                                .arg(&wd)
                                .args(["--spool"])
                                .arg(run.join(format!("emlwatch-{}", name)))
                                .stdin(Stdio::null())
                                .spawn()
                                .expect("spawn emlwatch"),
                        );
                    }
                }
            }

            if children.is_empty() {
                println!("emlctl: nothing to start (no services/ jobs/ watches/)");
                std::process::exit(0);
            }
            println!("emlctl: {} component(s) up", children.len());

            if let Some(m) = max_runtime {
                std::thread::sleep(std::time::Duration::from_secs(m));
                for c in children.iter_mut() {
                    libc_kill(c.id() as i32);
                    let _ = c.wait();
                }
                println!("emlctl: stopped after {}s", m);
                0
            } else {
                // wait for any child to exit, then stop the rest
                loop {
                    let mut any_done = false;
                    for c in children.iter_mut() {
                        if let Ok(Some(_)) = c.try_wait() {
                            any_done = true;
                        }
                    }
                    if any_done {
                        for c in children.iter_mut() {
                            libc_kill(c.id() as i32);
                            let _ = c.wait();
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                0
            }
        }
        _ => {
            eprintln!("usage: emlctl doctor|status|verify|up [--root DIR] [--max-runtime SECS]");
            2
        }
    };
    std::process::exit(code);
}

fn libc_kill(pid: i32) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}
