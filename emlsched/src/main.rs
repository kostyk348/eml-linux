//! emlsched CLI — CPU load balancing advisor.
//!
//!   emlsched sample [--interval-ms N] [--spool DIR]
//!   emlsched plan   --loads "..." [--tasks N] [--decay D] [--steps S] [--spool DIR]
//!   emlsched daemon [--spool DIR] [--interval SECS] [--max-runtime SECS]
//!   emlsched pin    --pid P --cpu C

use emlcore::Spool;
use emlsched::{busy_percent, parse_cpu_times, plan, set_affinity};
use std::time::Duration;

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

fn read_cpu_times() -> Vec<emlsched::CpuTimes> {
    let text = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    parse_cpu_times(&text)
}

fn sample_loads(interval_ms: u64) -> Vec<u8> {
    let prev = read_cpu_times();
    std::thread::sleep(Duration::from_millis(interval_ms));
    let cur = read_cpu_times();
    busy_percent(&prev, &cur)
}

fn bar(v: u8) -> String {
    let n = (v as usize * 20) / 255;
    format!("{}{}", "#".repeat(n), ".".repeat(20 - n))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("sample");
    let code = match cmd {
        "sample" => {
            let ms: u64 = flag(&args, "--interval-ms").and_then(|s| s.parse().ok()).unwrap_or(200);
            let loads = sample_loads(ms);
            for (i, v) in loads.iter().enumerate() {
                println!("cpu{:<3} {:>3}  {}", i, v, bar(*v));
            }
            if let Some(spool_dir) = flag(&args, "--spool") {
                if let Ok(mut s) = Spool::open(&spool_dir) {
                    let _ = s.append("SCHED_SAMPLE", "emlsched", serde_json::json!({"loads": loads}));
                }
            }
            0
        }
        "plan" => {
            let loads: Vec<u8> = flag(&args, "--loads")
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if loads.is_empty() {
                eprintln!("emlsched plan --loads \"10,80,20,0\" [--tasks 4] [--decay 1] [--steps 3]");
                std::process::exit(2);
            }
            let tasks: usize = flag(&args, "--tasks").and_then(|s| s.parse().ok()).unwrap_or(4);
            let decay: u8 = flag(&args, "--decay").and_then(|s| s.parse().ok()).unwrap_or(1);
            let steps: usize = flag(&args, "--steps").and_then(|s| s.parse().ok()).unwrap_or(3);
            let (diff, place, pay) = plan(&loads, tasks, decay, steps);
            println!("loads:    {:?}", loads);
            println!("diffused: {:?}", diff);
            for (t, (c, p)) in place.iter().zip(&pay).enumerate() {
                println!("task {} -> cpu {} (vcg cost {})", t, c, p);
            }
            if let Some(spool_dir) = flag(&args, "--spool") {
                if let Ok(mut s) = Spool::open(&spool_dir) {
                    let _ = s.append(
                        "SCHED_PLAN",
                        "emlsched",
                        serde_json::json!({"loads": loads, "diffused": diff, "placement": place, "payments": pay}),
                    );
                }
            }
            0
        }
        "daemon" => {
            let spool_dir = flag(&args, "--spool").unwrap_or_else(|| "run/emlsched".into());
            let interval: u64 = flag(&args, "--interval").and_then(|s| s.parse().ok()).unwrap_or(1);
            let max_runtime: Option<u64> = flag(&args, "--max-runtime").and_then(|s| s.parse().ok());
            let mut spool = match Spool::open(&spool_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("emlsched: {}", e);
                    std::process::exit(1);
                }
            };
            let start = std::time::Instant::now();
            println!("emlsched: sampling every {}s -> {}", interval, spool_dir);
            loop {
                let loads = sample_loads(200);
                let (diff, place, pay) = plan(&loads, 4, 1, 3);
                let _ = spool.append(
                    "SCHED_PLAN",
                    "emlsched",
                    serde_json::json!({"loads": loads, "diffused": diff, "placement": place, "payments": pay}),
                );
                let avg: u32 = if loads.is_empty() {
                    0
                } else {
                    loads.iter().map(|&x| x as u32).sum::<u32>() / loads.len() as u32
                };
                println!("avg load {} -> tasks placed on {:?}", avg, place);
                if let Some(m) = max_runtime {
                    if start.elapsed().as_secs() >= m {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
            let st = spool.verify();
            println!("emlsched: done ({} events, chain {})", spool.len(), if st.ok { "ok" } else { "BROKEN" });
            0
        }
        "pin" => {
            let pid: i32 = flag(&args, "--pid").and_then(|s| s.parse().ok()).unwrap_or(0);
            let cpu: usize = flag(&args, "--cpu").and_then(|s| s.parse().ok()).unwrap_or(0);
            match set_affinity(pid, cpu) {
                Ok(()) => {
                    println!("pinned pid {} -> cpu {}", pid, cpu);
                    0
                }
                Err(e) => {
                    eprintln!("emlsched: pin failed: {}", e);
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: emlsched sample|plan|daemon|pin ...");
            2
        }
    };
    std::process::exit(code);
}
