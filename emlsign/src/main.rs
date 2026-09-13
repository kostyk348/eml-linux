//! emlsign CLI — ed25519 provenance for .eml records.
//!
//!   emlsign keygen --key FILE
//!   emlsign pub    --key FILE
//!   emlsign sign   --key FILE --in FILE [--out FILE]
//!   emlsign verify --in FILE [--pub HEX]

use emlcore::Record;
use emlsign::{from_hex, generate, sign_record, to_hex, verify_record};
use ed25519_dalek::SigningKey;
use std::fs;

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

fn load_key(path: &str) -> Result<SigningKey, String> {
    let hex = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let bytes = from_hex(hex.trim())?;
    if bytes.len() != 32 {
        return Err("key must be 32 bytes".into());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&arr))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("verify");
    let code = match cmd {
        "keygen" => {
            let key = flag(&args, "--key").unwrap_or_else(|| "emlsign.key".into());
            let sk = generate();
            if let Err(e) = fs::write(&key, to_hex(&sk.to_bytes()) + "\n") {
                eprintln!("emlsign: {}", e);
                std::process::exit(1);
            }
            println!("pub {}", to_hex(sk.verifying_key().as_bytes()));
            0
        }
        "pub" => {
            let key = flag(&args, "--key").unwrap_or_else(|| "emlsign.key".into());
            match load_key(&key) {
                Ok(sk) => {
                    println!("{}", to_hex(sk.verifying_key().as_bytes()));
                    0
                }
                Err(e) => {
                    eprintln!("emlsign: {}", e);
                    1
                }
            }
        }
        "sign" => {
            let key = flag(&args, "--key").unwrap_or_else(|| "emlsign.key".into());
            let input = match flag(&args, "--in") {
                Some(p) => p,
                None => {
                    eprintln!("emlsign sign --key FILE --in FILE [--out FILE]");
                    std::process::exit(2);
                }
            };
            let sk = match load_key(&key) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("emlsign: {}", e);
                    std::process::exit(1);
                }
            };
            let data = match fs::read(&input) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("emlsign: {}", e);
                    std::process::exit(1);
                }
            };
            let mut rec = Record::parse(&data);
            sign_record(&sk, &mut rec);
            let out = flag(&args, "--out").unwrap_or(input);
            if let Err(e) = fs::write(&out, rec.render()) {
                eprintln!("emlsign: {}", e);
                std::process::exit(1);
            }
            println!("signed {} -> {}", out, to_hex(sk.verifying_key().as_bytes()));
            0
        }
        "verify" => {
            let input = match flag(&args, "--in") {
                Some(p) => p,
                None => {
                    eprintln!("emlsign verify --in FILE [--pub HEX]");
                    std::process::exit(2);
                }
            };
            let data = match fs::read(&input) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("emlsign: {}", e);
                    std::process::exit(1);
                }
            };
            let rec = Record::parse(&data);
            let expect = flag(&args, "--pub").and_then(|h| {
                from_hex(&h).ok().and_then(|b| {
                    if b.len() == 32 {
                        let mut a = [0u8; 32];
                        a.copy_from_slice(&b);
                        Some(a)
                    } else {
                        None
                    }
                })
            });
            match verify_record(&rec, expect) {
                Ok(()) => {
                    println!("OK: signature valid ({})", rec.get("X-Sign-Pub").unwrap_or(""));
                    0
                }
                Err(e) => {
                    println!("INVALID: {}", e);
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: emlsign keygen|pub|sign|verify ...");
            2
        }
    };
    std::process::exit(code);
}
