//! emlsched — CPU load balancing via fuzzy CA diffusion and a congestion game.
//!
//! * `/proc/stat` is sampled into per-CPU busy fractions (0..255 fuzzy scale).
//! * `emlca::fuzzy_diffuse` smooths the load field over the CPU ring.
//! * `vcg_placement` assigns tasks to the coolest cells and charges each task
//!   the marginal increase in congestion cost (sum of squared loads).

use emlca::fuzzy_diffuse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// Parse `/proc/stat` "cpuN" lines into per-CPU cumulative times.
pub fn parse_cpu_times(text: &str) -> Vec<CpuTimes> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let name = match it.next() {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("cpu") || name == "cpu" {
            continue;
        }
        let vals: Vec<u64> = it.filter_map(|x| x.parse().ok()).collect();
        if vals.len() < 5 {
            continue;
        }
        let total: u64 = vals.iter().sum();
        let idle = vals[3] + vals[4]; // idle + iowait
        out.push(CpuTimes { idle, total });
    }
    out
}

/// Busy fraction per CPU between two samples, scaled to 0..255 (fuzzy units).
pub fn busy_percent(prev: &[CpuTimes], cur: &[CpuTimes]) -> Vec<u8> {
    prev.iter()
        .zip(cur)
        .map(|(p, c)| {
            let dt = c.total.saturating_sub(p.total);
            if dt == 0 {
                return 0;
            }
            let di = c.idle.saturating_sub(p.idle);
            let busy = dt.saturating_sub(di);
            ((busy as f64 / dt as f64) * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// Diffuse the load field, then place `tasks` on the coolest cells.
/// Returns (diffused field, chosen cell per task, per-task VCG payment).
pub fn plan(
    loads: &[u8],
    tasks: usize,
    decay: u8,
    steps: usize,
) -> (Vec<u8>, Vec<usize>, Vec<u64>) {
    let diffused = fuzzy_diffuse(loads, decay, steps);
    let (placement, payments) = vcg_placement(&diffused, tasks);
    (diffused, placement, payments)
}

/// Congestion-game placement: cost = sum(load^2). Greedy assigns each task to
/// the cell minimising the new cost; each task pays the marginal cost it adds.
pub fn vcg_placement(loads: &[u8], tasks: usize) -> (Vec<usize>, Vec<u64>) {
    let mut cur: Vec<u64> = loads.iter().map(|&x| x as u64).collect();
    let mut placement = Vec::with_capacity(tasks);
    let mut payments = Vec::with_capacity(tasks);
    if cur.is_empty() {
        return (placement, payments);
    }
    for _ in 0..tasks {
        let mut best = 0usize;
        let mut best_delta = u64::MAX;
        for (i, &l) in cur.iter().enumerate() {
            let delta = (l + 1) * (l + 1) - l * l;
            if delta < best_delta {
                best_delta = delta;
                best = i;
            }
        }
        cur[best] += 1;
        placement.push(best);
        payments.push(best_delta);
    }
    (placement, payments)
}

/// Apply CPU affinity for a process (Linux `sched_setaffinity`).
pub fn set_affinity(pid: i32, cpu: usize) -> Result<(), String> {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
        let rc = libc::sched_setaffinity(pid, std::mem::size_of::<libc::cpu_set_t>(), &set);
        if rc != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "cpu  100 0 50 1000 10 0 0 0 0 0\ncpu0 10 0 5 100 1 0 0 0 0 0\ncpu1 20 0 10 200 2 0 0 0 0 0\n";

    #[test]
    fn parses_per_cpu() {
        let t = parse_cpu_times(SAMPLE);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].total, 116);
        assert_eq!(t[0].idle, 101);
        assert_eq!(t[1].total, 232);
    }

    #[test]
    fn busy_fraction() {
        let prev = vec![CpuTimes { idle: 0, total: 0 }];
        let cur = vec![CpuTimes { idle: 50, total: 100 }];
        assert_eq!(busy_percent(&prev, &cur), vec![128]);
        let idle = vec![CpuTimes { idle: 100, total: 100 }];
        assert_eq!(busy_percent(&prev, &idle), vec![0]);
    }

    #[test]
    fn placement_fills_coolest_first() {
        let (place, pay) = vcg_placement(&[0, 200], 2);
        assert_eq!(place, vec![0, 0]);
        assert_eq!(pay, vec![1, 3]);
    }

    #[test]
    fn plan_diffuses_and_places() {
        let (diff, place, pay) = plan(&[255, 0, 0, 0], 1, 1, 3);
        assert_eq!(diff[0], 255);
        assert_eq!(place.len(), 1);
        assert_eq!(pay.len(), 1);
        let coolest = diff
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(place[0], coolest);
    }
}
