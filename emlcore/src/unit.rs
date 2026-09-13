//! Service unit parsed from an `.eml` container.
//!
//! Minimal, readable definition:
//!
//! ```text
//! From: <svc@eml.local>
//! To: <init@eml.local>
//! Subject: my service
//! X-EML-Type: Application/Service
//! X-Entity-ID: my-service
//! Content-Type: application/json
//!
//! {"exec":["/bin/sleep","5"],"restart":"on-failure","restart_sec":1,"after":[],"env":{}}
//! ```
//!
//! Headers (`X-Init-Exec`, `X-Init-Restart`, `X-Init-After`, `X-Init-Enabled`)
//! are accepted as fallbacks so a unit can be written with headers only.

use crate::record::Record;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

impl RestartPolicy {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" | "yes" | "true" => Self::Always,
            "on-failure" | "on_failure" | "failure" => Self::OnFailure,
            _ => Self::Never,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::OnFailure => "on-failure",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: String,
    pub description: String,
    pub exec: Vec<String>,
    pub restart: RestartPolicy,
    pub restart_secs: f64,
    pub after: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub enabled: bool,
    pub source: PathBuf,
}

impl Unit {
    pub fn from_record(rec: &Record, path: &Path) -> Result<Unit, String> {
        let body: Value = serde_json::from_slice(&rec.body).unwrap_or(Value::Null);

        let id = rec
            .get("X-Entity-ID")
            .or_else(|| rec.get("Subject"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unnamed".into())
            });

        let mut exec: Vec<String> = Vec::new();
        if let Some(arr) = body.get("exec").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    exec.push(s.to_string());
                }
            }
        } else if let Some(s) = rec
            .get("X-Init-Exec")
            .or_else(|| body.get("exec").and_then(|v| v.as_str()))
        {
            exec = s.split_whitespace().map(|x| x.to_string()).collect();
        }
        if exec.is_empty() {
            return Err(format!("unit {}: empty exec", id));
        }

        let restart = rec
            .get("X-Init-Restart")
            .map(RestartPolicy::parse)
            .or_else(|| {
                body.get("restart")
                    .and_then(|v| v.as_str())
                    .map(RestartPolicy::parse)
            })
            .unwrap_or(RestartPolicy::OnFailure);

        let restart_secs = rec
            .get("X-Init-Restart-Sec")
            .and_then(|s| s.parse().ok())
            .or_else(|| body.get("restart_sec").and_then(|v| v.as_f64()))
            .unwrap_or(1.0);

        let after: Vec<String> = rec
            .get("X-Init-After")
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .or_else(|| {
                body.get("after").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
            })
            .unwrap_or_default();

        let mut env = BTreeMap::new();
        if let Some(obj) = body.get("env").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    env.insert(k.clone(), s.to_string());
                }
            }
        }

        let enabled = rec
            .get("X-Init-Enabled")
            .map(|s| !matches!(s.trim().to_ascii_lowercase().as_str(), "false" | "no" | "0"))
            .or_else(|| body.get("enabled").and_then(|v| v.as_bool()))
            .unwrap_or(true);

        let description = rec.get("Subject").unwrap_or("").to_string();

        Ok(Unit {
            id,
            description,
            exec,
            restart,
            restart_secs,
            after,
            env,
            enabled,
            source: path.to_path_buf(),
        })
    }

    pub fn load_dir(dir: &Path) -> Result<Vec<Unit>, String> {
        let mut out = Vec::new();
        let rd = std::fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e))?;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "eml").unwrap_or(false) {
                let data = std::fs::read(&p).map_err(|e| e.to_string())?;
                let rec = Record::parse(&data);
                out.push(Unit::from_record(&rec, &p)?);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Record {
        Record::parse(s.as_bytes())
    }

    #[test]
    fn parses_json_body() {
        let rec = parse(
            "From: <svc@eml.local>\nX-Entity-ID: web\nSubject: web server\nContent-Type: application/json\n\n{\"exec\":[\"/bin/sh\",\"-c\",\"echo hi\"],\"restart\":\"always\",\"restart_sec\":2,\"after\":[\"net\"],\"env\":{\"PORT\":\"80\"}}",
        );
        let u = Unit::from_record(&rec, Path::new("web.eml")).unwrap();
        assert_eq!(u.id, "web");
        assert_eq!(u.exec, vec!["/bin/sh", "-c", "echo hi"]);
        assert_eq!(u.restart, RestartPolicy::Always);
        assert_eq!(u.restart_secs, 2.0);
        assert_eq!(u.after, vec!["net"]);
        assert_eq!(u.env.get("PORT"), Some(&"80".to_string()));
        assert!(u.enabled);
    }

    #[test]
    fn parses_header_fallback() {
        let rec = parse(
            "X-Entity-ID: t\nX-Init-Exec: /bin/true\nX-Init-Restart: never\nX-Init-After: a,b\n\n",
        );
        let u = Unit::from_record(&rec, Path::new("t.eml")).unwrap();
        assert_eq!(u.exec, vec!["/bin/true"]);
        assert_eq!(u.restart, RestartPolicy::Never);
        assert_eq!(u.after, vec!["a", "b"]);
    }

    #[test]
    fn rejects_empty_exec() {
        let rec = parse("X-Entity-ID: t\n\n{}");
        assert!(Unit::from_record(&rec, Path::new("t.eml")).is_err());
    }
}
