//! emlcron — interval scheduler over `.eml` job units.
//!
//! A job is a `.eml` record with `{"exec":[...],"every_sec":N}`. The scheduler
//! fires due jobs and records RUN/EXIT as hash-chained `.eml` events.

use emlcore::Record;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub exec: Vec<String>,
    pub every: Duration,
    pub env: BTreeMap<String, String>,
    pub source: PathBuf,
}

impl Job {
    pub fn from_record(rec: &Record, path: &Path) -> Result<Job, String> {
        let body: Value = serde_json::from_slice(&rec.body).unwrap_or(Value::Null);
        let id = rec
            .get("X-Entity-ID")
            .or_else(|| rec.get("Subject"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "job".into())
            });

        let mut exec = Vec::new();
        if let Some(a) = body.get("exec").and_then(|v| v.as_array()) {
            for v in a {
                if let Some(s) = v.as_str() {
                    exec.push(s.to_string());
                }
            }
        } else if let Some(s) = rec.get("X-Cron-Exec") {
            exec = s.split_whitespace().map(|x| x.to_string()).collect();
        }
        if exec.is_empty() {
            return Err(format!("job {}: empty exec", id));
        }

        let every = if let Some(ms) = body.get("every_ms").and_then(|v| v.as_u64()) {
            Duration::from_millis(ms)
        } else if let Some(sec) = body.get("every_sec").and_then(|v| v.as_f64()) {
            Duration::from_secs_f64(sec.max(0.001))
        } else if let Some(sec) = rec.get("X-Cron-Every-Sec").and_then(|s| s.parse::<f64>().ok()) {
            Duration::from_secs_f64(sec.max(0.001))
        } else {
            Duration::from_secs(60)
        };

        let mut env = BTreeMap::new();
        if let Some(o) = body.get("env").and_then(|v| v.as_object()) {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    env.insert(k.clone(), s.to_string());
                }
            }
        }

        Ok(Job {
            id,
            exec,
            every,
            env,
            source: path.to_path_buf(),
        })
    }

    pub fn load_dir(dir: &Path) -> Result<Vec<Job>, String> {
        let mut out = Vec::new();
        let rd = std::fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e))?;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "eml").unwrap_or(false) {
                let data = std::fs::read(&p).map_err(|e| e.to_string())?;
                out.push(Job::from_record(&Record::parse(&data), &p)?);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }
}

pub struct Scheduler {
    jobs: Vec<Job>,
    next: Vec<Instant>,
}

impl Scheduler {
    pub fn new(jobs: Vec<Job>, now: Instant) -> Self {
        let next = jobs.iter().map(|j| now + j.every).collect();
        Scheduler { jobs, next }
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn job(&self, i: usize) -> &Job {
        &self.jobs[i]
    }

    /// Return indices of jobs due at `now`, and reschedule them.
    pub fn tick(&mut self, now: Instant) -> Vec<usize> {
        let mut due = Vec::new();
        for i in 0..self.jobs.len() {
            if now >= self.next[i] {
                due.push(i);
                self.next[i] = now + self.jobs[i].every;
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, ms: u64) -> Job {
        Job {
            id: id.into(),
            exec: vec!["/bin/true".into()],
            every: Duration::from_millis(ms),
            env: BTreeMap::new(),
            source: PathBuf::from("x.eml"),
        }
    }

    #[test]
    fn fires_only_when_due() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(vec![job("a", 100)], t0);
        assert!(s.tick(t0).is_empty());
        assert!(s.tick(t0 + Duration::from_millis(50)).is_empty());
        assert_eq!(s.tick(t0 + Duration::from_millis(100)), vec![0usize]);
        assert!(s.tick(t0 + Duration::from_millis(150)).is_empty());
        assert_eq!(s.tick(t0 + Duration::from_millis(200)), vec![0usize]);
    }

    #[test]
    fn parses_job() {
        let rec = Record::parse(
            b"X-Entity-ID: ping\nContent-Type: application/json\n\n{\"exec\":[\"/bin/echo\",\"hi\"],\"every_sec\":2.5}",
        );
        let j = Job::from_record(&rec, Path::new("ping.eml")).unwrap();
        assert_eq!(j.id, "ping");
        assert_eq!(j.exec, vec!["/bin/echo", "hi"]);
        assert_eq!(j.every, Duration::from_secs_f64(2.5));
    }
}
