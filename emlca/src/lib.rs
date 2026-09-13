//! emlca — fuzzy cellular automata + game theory for Linux resource governance.
//!
//! Non-AI framing: cells are workers/queues, load diffuses as a fuzzy field,
//! and scarce resources (slots, credit, arbitration) are allocated by
//! incentive-compatible mechanisms (VCG, Shapley, Nash bargaining).

use std::collections::HashMap;

/// One fuzzy step on a ring: `next[i] = max(self, max(neighbours) - decay)`.
pub fn fuzzy_diffuse_step(cells: &[u8], decay: u8) -> Vec<u8> {
    let n = cells.len();
    if n == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; n];
    for i in 0..n {
        let l = cells[(i + n - 1) % n];
        let r = cells[(i + 1) % n];
        let nm = l.max(r);
        let prop = nm.saturating_sub(decay);
        out[i] = cells[i].max(prop);
    }
    out
}

pub fn fuzzy_diffuse(cells: &[u8], decay: u8, steps: usize) -> Vec<u8> {
    let mut cur = cells.to_vec();
    for _ in 0..steps {
        cur = fuzzy_diffuse_step(&cur, decay);
    }
    cur
}

/// VCG auction for `slots` identical items, unit demand.
/// Returns (winners, payments). Each winner pays the first losing bid.
pub fn vcg_auction(bids: &[u64], slots: usize) -> (Vec<usize>, Vec<u64>) {
    let mut idx: Vec<usize> = (0..bids.len()).collect();
    idx.sort_by(|&a, &b| bids[b].cmp(&bids[a]));
    if slots == 0 || slots >= bids.len() {
        let winners = idx.into_iter().take(slots.min(bids.len())).collect::<Vec<_>>();
        let payments = vec![0u64; winners.len()];
        return (winners, payments);
    }
    let threshold = bids[idx[slots]];
    let winners: Vec<usize> = idx.into_iter().take(slots).collect();
    let payments = vec![threshold; winners.len()];
    (winners, payments)
}

/// Shapley value from a characteristic function keyed by coalition bitmask.
/// `players` order defines bit positions (bit i = player i present).
pub fn shapley(players: usize, v: &HashMap<u64, f64>) -> Vec<f64> {
    let n = players;
    if n == 0 {
        return Vec::new();
    }
    let factorial = |k: usize| -> f64 { (1..=k).map(|x| x as f64).product::<f64>().max(1.0) };
    let nf = factorial(n);
    let mut phi = vec![0.0f64; n];
    for i in 0..n {
        let mut acc = 0.0;
        // iterate all subsets S not containing i
        for mask in 0u64..(1u64 << n) {
            if mask & (1u64 << i) != 0 {
                continue;
            }
            let s = mask.count_ones() as usize;
            let weight = factorial(s) * factorial(n - s - 1) / nf;
            let with = v.get(&(mask | (1u64 << i))).copied().unwrap_or(0.0);
            let without = v.get(&mask).copied().unwrap_or(0.0);
            acc += weight * (with - without);
        }
        phi[i] = acc;
    }
    phi
}

/// Nash bargaining: argmax over candidates of `sum ln(u_i - d_i)`.
/// Candidates with any `u_i <= d_i` are infeasible.
pub fn nash_bargain(candidates: &[Vec<f64>], disagreement: &[f64]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (ci, c) in candidates.iter().enumerate() {
        if c.len() != disagreement.len() {
            continue;
        }
        let mut score = 0.0;
        let mut ok = true;
        for (u, d) in c.iter().zip(disagreement) {
            let surplus = u - d;
            if surplus <= 0.0 {
                ok = false;
                break;
            }
            score += surplus.ln();
        }
        if ok && best.map(|(_, b)| score > b).unwrap_or(true) {
            best = Some((ci, score));
        }
    }
    best.map(|(i, _)| i)
}

/// Build a coalition-value map from `A=1,B=2,AB=5,...` keys.
pub fn parse_coalitions(players: &[String], spec: &str) -> Result<HashMap<u64, f64>, String> {
    let mut m = HashMap::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (coal, val) = part
            .split_once('=')
            .ok_or_else(|| format!("bad coalition spec: {}", part))?;
        let val: f64 = val.trim().parse().map_err(|e| format!("{}: {}", part, e))?;
        let mut mask = 0u64;
        for ch in coal.trim().chars() {
            let name = ch.to_string();
            let i = players
                .iter()
                .position(|p| p.eq_ignore_ascii_case(&name))
                .ok_or_else(|| format!("unknown player '{}'", name))?;
            mask |= 1u64 << i;
        }
        m.insert(mask, val);
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffusion_spreads_and_decays() {
        let out = fuzzy_diffuse(&[255, 0, 0, 0], 1, 3);
        assert_eq!(out[0], 255);
        // on a 4-ring the seed reaches every cell
        assert!(out[1] > 0 && out[2] > 0 && out[3] > 0);
        // symmetric neighbours stay equal
        assert_eq!(out[1], out[3]);
        assert!(out.iter().all(|&x| x <= 255));
    }

    #[test]
    fn vcg_pays_first_losing_bid() {
        let (w, p) = vcg_auction(&[10, 7, 3], 2);
        assert_eq!(w, vec![0, 1]);
        assert_eq!(p, vec![3, 3]);
        // one slot: winner pays second bid
        let (w1, p1) = vcg_auction(&[10, 7, 3], 1);
        assert_eq!(w1, vec![0]);
        assert_eq!(p1, vec![7]);
    }

    #[test]
    fn shapley_symmetric_game() {
        // 3 players, v(S)=|S|^2 -> symmetric; each phi = v(N)/n = 9/3 = 3
        let mut v = HashMap::new();
        v.insert(0, 0.0);
        for mask in 1u64..8 {
            let k = mask.count_ones() as f64;
            v.insert(mask, k * k);
        }
        let phi = shapley(3, &v);
        let sum: f64 = phi.iter().sum();
        assert!((sum - 9.0).abs() < 1e-9, "efficiency broken: {}", sum);
        for x in phi {
            assert!((x - 3.0).abs() < 1e-9, "got {}", x);
        }
    }

    #[test]
    fn nash_protects_the_weak() {
        // candidate 0: (10,1) candidate 1: (6,6), d=(0,0)
        // log-sum: ln10+ln1=2.30 ; ln6+ln6=3.58 -> picks fair split
        let i = nash_bargain(&[vec![10.0, 1.0], vec![6.0, 6.0]], &[0.0, 0.0]);
        assert_eq!(i, Some(1));
    }

    #[test]
    fn nash_rejects_infeasible() {
        let i = nash_bargain(&[vec![0.5, 5.0]], &[1.0, 1.0]);
        assert_eq!(i, None);
    }
}
