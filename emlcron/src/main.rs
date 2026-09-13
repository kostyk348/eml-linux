//! emlcron CLI — run interval jobs defined as .eml units.
//!
//!   emlcron list --jobs DIR
//!   emlcron run  --jobs DIR --spool DIR [--max-runtime SECS]

use emlcore::Spool;
use emlcron::{Job, Scheduler};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("list");
    let jobs_dir = PathBuf::from(flag(&args, "--jobs").unwrap_or_else(|| "jobs".into()));
    let code = match cmd {
        "list" => match Job::load_dir(&jobs_dir) {
            Ok(jobs) => {
                for j in jobs {
                    println!(
                        "{:<16} every={:?} {}",
                        j.id,
                        j.every,
                        j.exec.join(" ")
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("emlcron: {}", e);
                1
            }
        },
        "run" => {
            let spool_dir = flag(&args, "--spool").unwrap_or_else(|| "run/emlcron".into());
            let max_runtime: Option<u64> = flag(&args, "--max-runtime").and_then(|s| s.parse().ok());
            let jobs = match Job::load_dir(&jobs_dir) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("emlcron: {}", e);
                    std::process::exit(1);
                }
            };
            let mut spool = match Spool::open(&spool_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlcron: {}", e);
                    std::process::exit(1);
                }
            };
            let _ = spool.append(
                "CRON_BOOT",
                "emlcron",
                serde_json::json!({"jobs": jobs.len(), "pid": std::process::id()}),
            );
            println!("emlcron: {} job(s), spool {}", jobs.len(), spool_dir);

            let start = Instant::now();
            let mut sched = Scheduler::new(jobs, start);
            let mut runs = 0u64;
            loop {
                let now = Instant::now();
                for i in sched.tick(now) {
                    let job = sched.job(i).clone();
                    runs += 1;
                    let _ = spool.append(
                        "CRON_RUN",
                        "emlcron",
                        serde_json::json!({"job": job.id, "run": runs, "exec": job.exec}),
                    );
                    let mut cmd = Command::new(&job.exec[0]);
                    cmd.args(&job.exec[1..]);
                    for (k, v) in &job.env {
                        cmd.env(k, v);
                    }
                    cmd.stdin(Stdio::null());
                    match cmd.output() {
                        Ok(out) => {
                            let _ = spool.append(
                                "CRON_EXIT",
                                "emlcron",
                                serde_json::json!({
                                    "job": job.id, "code": out.status.code(),
                                    "stdout": String::from_utf8_lossy(&out.stdout).trim()
                                }),
                            );
                            println!("ran {} (code {:?})", job.id, out.status.code());
                        }
                        Err(e) => {
                            let _ = spool.append(
                                "CRON_ERROR",
                                "emlcron",
                                serde_json::json!({"job": job.id, "error": e.to_string()}),
                            );
                            println!("failed {}: {}", job.id, e);
                        }
                    }
                }
                if let Some(m) = max_runtime {
                    if start.elapsed().as_secs() >= m {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let st = spool.verify();
            println!("emlcron: done ({} runs, {} events, chain {})", runs, spool.len(), if st.ok { "ok" } else { "BROKEN" });
            if st.ok {
                0
            } else {
                1
            }
        }
        _ => {
            eprintln!("usage: emlcron list|run --jobs DIR [--spool DIR] [--max-runtime SECS]");
            2
        }
    };
    std::process::exit(code);
}

