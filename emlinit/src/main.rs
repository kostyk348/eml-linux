//! emlinit — an EML-native init / service supervisor.
//!
//!   emlinit run    [--units DIR] [--spool DIR] [--once] [--max-runtime SECS]
//!   emlinit list   [--units DIR]
//!   emlinit status [--spool DIR]
//!   emlinit verify [--spool DIR]

use emlcore::{RestartPolicy, Spool, Unit};
use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn install_signals() {
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as libc::sighandler_t);
        libc::signal(libc::SIGHUP, handle_signal as libc::sighandler_t);
    }
}

struct Svc {
    unit: Unit,
    child: Option<Child>,
    restarts: u32,
    started: Option<Instant>,
    next_restart: Option<Instant>,
    state: &'static str,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("status");
    let mut units_dir = PathBuf::from("units");
    let mut spool_dir = PathBuf::from("run/emlinit");
    let mut once = false;
    let mut max_runtime: Option<u64> = None;
    let mut control_dir: Option<PathBuf> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--units" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    units_dir = PathBuf::from(v);
                }
            }
            "--spool" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    spool_dir = PathBuf::from(v);
                }
            }
            "--once" => once = true,
            "--max-runtime" => {
                i += 1;
                max_runtime = args.get(i).and_then(|v| v.parse().ok());
            }
            "--control" => {
                i += 1;
                control_dir = args.get(i).map(PathBuf::from);
            }
            _ => {}
        }
        i += 1;
    }
    let code = match cmd {
        "run" => cmd_run(&units_dir, &spool_dir, once, max_runtime, control_dir),
        "list" => cmd_list(&units_dir),
        "status" => cmd_status(&spool_dir),
        "verify" => cmd_verify(&spool_dir),
        "send" => cmd_send(&args[2..]),
        _ => {
            eprintln!("usage: emlinit run|list|status|verify|send [--units DIR] [--spool DIR] [--control DIR] [--once] [--max-runtime SECS]");
            2
        }
    };
    std::process::exit(code);
}

fn log_event(spool: &mut Spool, kind: &str, source: &str, payload: serde_json::Value) {
    if let Err(e) = spool.append(kind, source, payload) {
        eprintln!("emlinit: log append failed: {}", e);
    }
}

fn cmd_list(units_dir: &Path) -> i32 {
    match Unit::load_dir(units_dir) {
        Ok(units) => {
            if units.is_empty() {
                println!("(no units in {})", units_dir.display());
            }
            for u in units {
                let flag = if u.enabled { "" } else { " [disabled]" };
                println!(
                    "{:<20} {:<10} {:<28} {}{}",
                    u.id,
                    u.restart.as_str(),
                    u.exec.join(" "),
                    u.description,
                    flag
                );
            }
            0
        }
        Err(e) => {
            eprintln!("emlinit: {}", e);
            1
        }
    }
}

fn cmd_status(spool_dir: &Path) -> i32 {
    let spool = match Spool::open(spool_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlinit: {}", e);
            return 1;
        }
    };
    let mut last: HashMap<String, (String, String)> = HashMap::new();
    for ev in spool.events() {
        if let Some(u) = ev.payload.get("unit").and_then(|v| v.as_str()) {
            last.insert(u.to_string(), (ev.kind.clone(), ev.ts.clone()));
        }
    }
    println!("spool: {} events, head {}", spool.len(), spool.head());
    let mut ids: Vec<_> = last.keys().cloned().collect();
    ids.sort();
    for id in ids {
        let (k, ts) = &last[&id];
        println!("{:<20} {:<8} {}", id, k, ts);
    }
    0
}

fn cmd_verify(spool_dir: &Path) -> i32 {
    let spool = match Spool::open(spool_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlinit: {}", e);
            return 1;
        }
    };
    let st = spool.verify();
    if st.ok {
        println!("OK: {} events, chain intact (head {})", st.checked, spool.head());
        0
    } else {
        println!("BROKEN at seq {:?} after {} events", st.broken_at, st.checked);
        1
    }
}

/// Stable topological order: a unit appears after everything it lists in `after`.
fn topo_order(units: &[Unit]) -> Vec<String> {
    let ids: Vec<String> = units.iter().map(|u| u.id.clone()).collect();
    let mut placed: Vec<String> = Vec::new();
    let mut remaining: Vec<Unit> = units.to_vec();
    while !remaining.is_empty() {
        let mut progressed = false;
        let mut i = 0;
        while i < remaining.len() {
            let deps_ok = remaining[i]
                .after
                .iter()
                .all(|d| !ids.contains(d) || placed.contains(d));
            if deps_ok {
                placed.push(remaining[i].id.clone());
                remaining.remove(i);
                progressed = true;
            } else {
                i += 1;
            }
        }
        if !progressed {
            for u in &remaining {
                placed.push(u.id.clone());
            }
            break;
        }
    }
    placed
}

fn start_service(svc: &mut Svc, spool: &mut Spool, once: bool) {
    let mut cmd = Command::new(&svc.unit.exec[0]);
    cmd.args(&svc.unit.exec[1..]);
    for (k, v) in &svc.unit.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            svc.child = Some(child);
            svc.started = Some(Instant::now());
            svc.next_restart = None;
            svc.state = "running";
            log_event(
                spool,
                "START",
                "emlinit",
                serde_json::json!({"unit": svc.unit.id, "pid": pid, "exec": svc.unit.exec, "restarts": svc.restarts}),
            );
            println!("emlinit: started {} (pid {})", svc.unit.id, pid);
        }
        Err(e) => {
            log_event(
                spool,
                "ERROR",
                "emlinit",
                serde_json::json!({"unit": svc.unit.id, "error": e.to_string()}),
            );
            println!("emlinit: failed to start {}: {}", svc.unit.id, e);
            if once {
                svc.state = "exited";
            } else {
                svc.restarts += 1;
                svc.state = "pending";
                svc.next_restart =
                    Some(Instant::now() + Duration::from_secs_f64(svc.unit.restart_secs.max(0.1)));
            }
        }
    }
}

fn shutdown_all(svcs: &mut [Svc], spool: &mut Spool) {
    for s in svcs.iter_mut() {
        if let Some(child) = s.child.as_mut() {
            let pid = child.id() as i32;
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut alive = 0;
        for s in svcs.iter_mut() {
            if let Some(child) = s.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        s.child = None;
                    }
                    Ok(None) => alive += 1,
                    Err(_) => alive += 1,
                }
            }
        }
        if alive == 0 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for s in svcs.iter_mut() {
        if let Some(child) = s.child.as_mut() {
            let pid = child.id() as i32;
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            let _ = child.wait();
            log_event(
                spool,
                "STOP",
                "emlinit",
                serde_json::json!({"unit": s.unit.id, "forced": true}),
            );
            s.child = None;
        } else if s.state == "running" {
            log_event(
                spool,
                "STOP",
                "emlinit",
                serde_json::json!({"unit": s.unit.id, "forced": false}),
            );
        }
    }
}

fn cmd_run(
    units_dir: &Path,
    spool_dir: &Path,
    once: bool,
    max_runtime: Option<u64>,
    control_dir: Option<PathBuf>,
) -> i32 {
    install_signals();
    let mut spool = match Spool::open(spool_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emlinit: {}", e);
            return 1;
        }
    };
    let units: Vec<Unit> = match Unit::load_dir(units_dir) {
        Ok(u) => u.into_iter().filter(|u| u.enabled).collect(),
        Err(e) => {
            eprintln!("emlinit: {}", e);
            return 1;
        }
    };
    log_event(
        &mut spool,
        "INIT",
        "emlinit",
        serde_json::json!({"event": "boot", "units": units.len(), "pid": std::process::id(), "once": once}),
    );
    println!("emlinit: {} unit(s), spool {}", units.len(), spool_dir.display());

    let order = topo_order(&units);
    let mut svcs: Vec<Svc> = Vec::new();
    for id in &order {
        if let Some(u) = units.iter().find(|u| &u.id == id) {
            svcs.push(Svc {
                unit: u.clone(),
                child: None,
                restarts: 0,
                started: None,
                next_restart: None,
                state: "pending",
            });
        }
    }

    let start_all = Instant::now();
    let poll = Duration::from_millis(40);
    let control = control_dir.unwrap_or_else(|| spool_dir.join("control"));
    let _ = std::fs::create_dir_all(&control);

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log_event(&mut spool, "SHUTDOWN", "emlinit", serde_json::json!({"reason": "signal"}));
            shutdown_all(&mut svcs, &mut spool);
            break;
        }
        if let Some(m) = max_runtime {
            if start_all.elapsed().as_secs() >= m {
                log_event(
                    &mut spool,
                    "SHUTDOWN",
                    "emlinit",
                    serde_json::json!({"reason": "max-runtime", "secs": m}),
                );
                shutdown_all(&mut svcs, &mut spool);
                break;
            }
        }

        if let Some((event, unit)) = poll_control(&control) {
            log_event(
                &mut spool,
                "CONTROL",
                "emlinit",
                serde_json::json!({"event": event, "unit": unit}),
            );
            match event.as_str() {
                "STOP" => {
                    if let Some(u) = &unit {
                        if let Some(s) = svcs.iter_mut().find(|s| &s.unit.id == u) {
                            if let Some(child) = s.child.as_mut() {
                                unsafe {
                                    libc::kill(child.id() as i32, libc::SIGTERM);
                                }
                            }
                            s.state = "exited";
                        }
                    } else {
                        log_event(
                            &mut spool,
                            "SHUTDOWN",
                            "emlinit",
                            serde_json::json!({"reason": "bus STOP"}),
                        );
                        shutdown_all(&mut svcs, &mut spool);
                        break;
                    }
                }
                "START" => {
                    if let Some(u) = &unit {
                        if let Some(s) = svcs.iter_mut().find(|s| &s.unit.id == u) {
                            s.state = "pending";
                            s.next_restart = None;
                        }
                    }
                }
                "RELOAD" => {
                    if let Ok(units) = Unit::load_dir(units_dir) {
                        for u in units.into_iter().filter(|u| u.enabled) {
                            if let Some(s) = svcs.iter_mut().find(|s| s.unit.id == u.id) {
                                s.unit = u;
                            } else {
                                svcs.push(Svc {
                                    unit: u,
                                    child: None,
                                    restarts: 0,
                                    started: None,
                                    next_restart: None,
                                    state: "pending",
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        for i in 0..svcs.len() {
            if svcs[i].child.is_some() || svcs[i].state != "pending" {
                continue;
            }
            let deps_ok = svcs[i].unit.after.iter().all(|dep| {
                svcs.iter()
                    .find(|s| &s.unit.id == dep)
                    .map(|s| s.child.is_some() || s.state == "running" || s.state == "exited")
                    .unwrap_or(true)
            });
            if !deps_ok {
                continue;
            }
            let now = Instant::now();
            if svcs[i].next_restart.map(|t| now >= t).unwrap_or(true) {
                start_service(&mut svcs[i], &mut spool, once);
            }
        }

        for i in 0..svcs.len() {
            let mut exited = None;
            if let Some(child) = svcs[i].child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    exited = Some(status);
                }
            }
            if let Some(status) = exited {
                let code = status.code();
                let sig = status.signal();
                let uptime = svcs[i].started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                svcs[i].child = None;
                log_event(
                    &mut spool,
                    "EXIT",
                    "emlinit",
                    serde_json::json!({"unit": svcs[i].unit.id, "code": code, "signal": sig, "uptime_sec": uptime, "restarts": svcs[i].restarts}),
                );
                println!(
                    "emlinit: {} exited (code={:?} sig={:?}) after {}s",
                    svcs[i].unit.id, code, sig, uptime
                );
                let restart = !once
                    && match svcs[i].unit.restart {
                        RestartPolicy::Always => true,
                        RestartPolicy::OnFailure => code.map(|c| c != 0).unwrap_or(true),
                        RestartPolicy::Never => false,
                    };
                if restart {
                    svcs[i].restarts += 1;
                    svcs[i].state = "pending";
                    svcs[i].next_restart = Some(
                        Instant::now()
                            + Duration::from_secs_f64(svcs[i].unit.restart_secs.max(0.0)),
                    );
                } else {
                    svcs[i].state = "exited";
                }
            }
        }

        let all_done =
            !svcs.is_empty() && svcs.iter().all(|s| s.child.is_none() && s.state == "exited");
        if all_done {
            break;
        }
        std::thread::sleep(poll);
    }

    let st = spool.verify();
    log_event(&mut spool, "HALT", "emlinit", serde_json::json!({"chain_ok": st.ok}));
    if !st.ok {
        eprintln!("emlinit: WARNING chain broken at {:?}", st.broken_at);
    }
    println!(
        "emlinit: done ({} events, chain {})",
        spool.len(),
        if st.ok { "ok" } else { "BROKEN" }
    );
    if st.ok {
        0
    } else {
        1
    }
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

/// Take the oldest unprocessed control message; mark it `.done`.
fn poll_control(dir: &Path) -> Option<(String, Option<String>)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "eml").unwrap_or(false))
        .collect();
    files.sort();
    let f = files.into_iter().next()?;
    let data = std::fs::read(&f).ok()?;
    let rec = emlcore::Record::parse(&data);
    let event = rec.get("X-Event").unwrap_or("").to_string();
    let unit = rec.get("X-Unit").map(|s| s.to_string());
    let _ = std::fs::rename(&f, f.with_extension("eml.done"));
    Some((event, unit))
}

fn cmd_send(args: &[String]) -> i32 {
    let bus = flag(args, "--bus").unwrap_or_else(|| "run/emlinit/control".into());
    let event = flag(args, "--event").unwrap_or_else(|| "RELOAD".into());
    let unit = flag(args, "--unit");
    let dir = PathBuf::from(&bus);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("emlinit: {}", e);
        return 1;
    }
    let mut rec = emlcore::Record::new();
    rec.set("From", "<cli@eml.local>");
    rec.set("To", "<init@eml.local>");
    rec.set("X-EMLBox-Msg", "v1");
    rec.set("X-Event", event.clone());
    if let Some(u) = &unit {
        rec.set("X-Unit", u.clone());
    }
    rec.set("Content-Type", "application/json");
    rec.body = b"{}\n".to_vec();
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = dir.join(format!("{:016x}.control.eml", n));
    if let Err(e) = std::fs::write(&p, rec.render()) {
        eprintln!("emlinit: {}", e);
        return 1;
    }
    println!(
        "sent {} {} -> {}",
        event,
        unit.unwrap_or_default(),
        p.display()
    );
    0
}
