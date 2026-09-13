//! emlbus CLI — publish / subscribe / list over a .eml bus.
//!
//!   emlbus pub  --bus DIR --from A --to B --event E [--body JSON]
//!   emlbus sub  --bus DIR --to B [--once|--follow]
//!   emlbus list --bus DIR [--to B]
//!   emlbus tail --bus DIR

use emlbus::Bus;

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
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("list");
    let bus_dir = flag(&args, "--bus").unwrap_or_else(|| "run/bus".into());
    let bus = match Bus::open(&bus_dir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("emlbus: {}", e);
            std::process::exit(1);
        }
    };
    let code = match cmd {
        "pub" => {
            let from = flag(&args, "--from").unwrap_or_else(|| "cli".into());
            let to = flag(&args, "--to").unwrap_or_else(|| "*".into());
            let event = flag(&args, "--event").unwrap_or_else(|| "EVENT".into());
            let body: serde_json::Value = flag(&args, "--body")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({}));
            match bus.publish(&from, &to, &event, body) {
                Ok(p) => {
                    println!("published -> {}", p.display());
                    0
                }
                Err(e) => {
                    eprintln!("emlbus: {}", e);
                    1
                }
            }
        }
        "list" => {
            let to = flag(&args, "--to").unwrap_or_else(|| "*".into());
            for m in bus.pending(&to) {
                println!(
                    "{:<9} {:<12} -> {:<12} {}",
                    m.event,
                    m.from,
                    m.to,
                    serde_json::to_string(&m.body).unwrap_or_default()
                );
            }
            0
        }
        "tail" => {
            for m in bus.pending("*") {
                println!(
                    "{:<9} {:<12} -> {:<12} {}",
                    m.event,
                    m.from,
                    m.to,
                    serde_json::to_string(&m.body).unwrap_or_default()
                );
            }
            0
        }
        "sub" => {
            let to = flag(&args, "--to").unwrap_or_else(|| "*".into());
            let once = has(&args, "--once");
            loop {
                if let Some(m) = bus.consume(&to) {
                    println!(
                        "{} {} <- {} {}",
                        m.event,
                        m.to,
                        m.from,
                        serde_json::to_string(&m.body).unwrap_or_default()
                    );
                    if once {
                        break;
                    }
                } else {
                    if once {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            0
        }
        _ => {
            eprintln!("usage: emlbus pub|sub|list|tail --bus DIR ...");
            2
        }
    };
    std::process::exit(code);
}
