//! emlca CLI — fuzzy CA diffusion and game-theoretic allocation.
//! Every decision is appended to a hash-chained `.eml` spool (auditable).

use emlca::{fuzzy_diffuse, nash_bargain, parse_coalitions, shapley, vcg_auction};
use emlcore::Spool;

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
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("diffuse");
    let spool_dir = flag(&args, "--spool");
    let mut spool = spool_dir.as_ref().and_then(|d| Spool::open(d).ok());

    let code = match cmd {
        "diffuse" => cmd_diffuse(&args, &mut spool),
        "vcg" => cmd_vcg(&args, &mut spool),
        "shapley" => cmd_shapley(&args, &mut spool),
        "nash" => cmd_nash(&args, &mut spool),
        _ => {
            eprintln!("usage: emlca diffuse|vcg|shapley|nash [--spool DIR] ...");
            2
        }
    };
    std::process::exit(code);
}

fn emit(spool: &mut Option<Spool>, kind: &str, payload: serde_json::Value) {
    if let Some(s) = spool.as_mut() {
        let _ = s.append(kind, "emlca", payload);
    }
}

fn cmd_diffuse(args: &[String], spool: &mut Option<Spool>) -> i32 {
    let cells: Vec<u8> = flag(args, "--cells")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if cells.is_empty() {
        eprintln!("emlca diffuse --cells \"1,5,0,3\" [--decay 1] [--steps 4]");
        return 2;
    }
    let decay: u8 = flag(args, "--decay").and_then(|s| s.parse().ok()).unwrap_or(1);
    let steps: usize = flag(args, "--steps").and_then(|s| s.parse().ok()).unwrap_or(4);
    println!("t=0  {:?}", cells);
    let mut cur = cells.clone();
    for t in 1..=steps {
        cur = emlca::fuzzy_diffuse_step(&cur, decay);
        println!("t={}  {:?}", t, cur);
    }
    let out = fuzzy_diffuse(&cells, decay, steps);
    emit(
        spool,
        "CA_DIFFUSE",
        serde_json::json!({"cells": cells, "decay": decay, "steps": steps, "result": out}),
    );
    0
}

fn cmd_vcg(args: &[String], spool: &mut Option<Spool>) -> i32 {
    let bids: Vec<u64> = flag(args, "--bids")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let slots: usize = flag(args, "--slots").and_then(|s| s.parse().ok()).unwrap_or(1);
    if bids.is_empty() {
        eprintln!("emlca vcg --bids \"10,7,3\" [--slots 2]");
        return 2;
    }
    let (winners, payments) = vcg_auction(&bids, slots);
    for (w, p) in winners.iter().zip(&payments) {
        println!("winner {} bid={} pays={}", w, bids[*w], p);
    }
    let total: u64 = payments.iter().sum();
    println!("revenue={}", total);
    emit(
        spool,
        "GAME_VCG",
        serde_json::json!({"bids": bids, "slots": slots, "winners": winners, "payments": payments}),
    );
    0
}

fn cmd_shapley(args: &[String], spool: &mut Option<Spool>) -> i32 {
    let players: Vec<String> = flag(args, "--players")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let spec = flag(args, "--v").unwrap_or_default();
    if players.is_empty() || spec.is_empty() {
        eprintln!("emlca shapley --players \"A,B,C\" --v \"A=1,AB=5,ABC=6\"");
        return 2;
    }
    let v = match parse_coalitions(&players, &spec) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("emlca: {}", e);
            return 1;
        }
    };
    let phi = shapley(players.len(), &v);
    for (p, x) in players.iter().zip(&phi) {
        println!("{:<8} phi={:.6}", p, x);
    }
    emit(
        spool,
        "GAME_SHAPLEY",
        serde_json::json!({"players": players, "values": phi}),
    );
    0
}

fn cmd_nash(args: &[String], spool: &mut Option<Spool>) -> i32 {
    let cand_spec = flag(args, "--candidates").unwrap_or_default();
    let d_spec = flag(args, "--d").unwrap_or_default();
    let candidates: Vec<Vec<f64>> = cand_spec
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|row| row.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .collect();
    let disagreement: Vec<f64> = d_spec
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    if candidates.is_empty() || disagreement.is_empty() {
        eprintln!("emlca nash --candidates \"10,1;6,6\" --d \"0,0\"");
        return 2;
    }
    match nash_bargain(&candidates, &disagreement) {
        Some(i) => {
            println!("nash optimum: candidate {} = {:?}", i, candidates[i]);
            emit(
                spool,
                "GAME_NASH",
                serde_json::json!({"chosen": i, "candidate": candidates[i], "disagreement": disagreement}),
            );
            0
        }
        None => {
            println!("no feasible agreement (all candidates at/below disagreement)");
            emit(spool, "GAME_NASH", serde_json::json!({"chosen": null}));
            1
        }
    }
}
