//! Activity log: one JSON line for each thing the person does in the app —
//! a catalog or Soulseek search, queueing and finishing a download on Deezer,
//! Usenet or Soulseek — in `logs/activity.YYYY-MM-DD.jsonl` (local date).
//!
//! It is a plain record for the person and for local tools that read it (such
//! as Watchme); nothing is uploaded. Auto-get's own provider searches are left
//! out (its download is logged), so the log reads as what the person did.
//!
//! Hooks may call `record` from inside hub locks, so recording
//! only enqueues; one writer thread appends. Every failure is swallowed.
use chrono::{Local, TimeZone};
use serde_json::{Map, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const KEEP_DAYS: u64 = 90;
const PREFIX: &str = "activity.";
const SUFFIX: &str = ".jsonl";

static RECORDER: OnceLock<mpsc::Sender<Value>> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Starts the writer thread. Later calls are ignored.
pub fn start(directory: PathBuf) {
    let (sender, receiver) = mpsc::channel::<Value>();
    if RECORDER.set(sender).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("flipside-activity".into())
        .spawn(move || {
            let _ = std::fs::create_dir_all(&directory);
            prune(&directory, SystemTime::now());
            for line in receiver {
                let _ = append(&directory, &line);
            }
        });
}

/// Records one activity. `fields` should be a JSON object; its keys are added
/// next to `ts`, `id` and `event`.
pub fn record(event: &str, fields: Value) {
    if let Some(sender) = RECORDER.get() {
        let _ = sender.send(line(event, fields, now_ms()));
    }
}

fn line(event: &str, fields: Value, ts: i64) -> Value {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut line = Map::new();
    line.insert("ts".into(), ts.into());
    // Unique across restarts: readers use it to skip lines they already have.
    line.insert(
        "id".into(),
        format!("{ts}-{}-{sequence}", std::process::id()).into(),
    );
    line.insert("event".into(), event.into());
    if let Value::Object(fields) = fields {
        for (key, value) in fields {
            if !value.is_null() {
                line.insert(key, value);
            }
        }
    }
    Value::Object(line)
}

fn append(directory: &Path, line: &Value) -> std::io::Result<()> {
    let ts = line["ts"].as_i64().unwrap_or_default();
    let day = Local
        .timestamp_millis_opt(ts)
        .single()
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d");
    let path = directory.join(format!("{PREFIX}{day}{SUFFIX}"));
    // One write per line, so a reader never sees half a line followed by another.
    let mut text = serde_json::to_string(line)?;
    text.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

fn prune(directory: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let cutoff = now - Duration::from_secs(KEEP_DAYS * 24 * 60 * 60);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with(PREFIX) && name.ends_with(SUFFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|modified| modified < cutoff);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "flipside-activity-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lines_carry_time_id_event_and_fields_without_nulls() {
        let value = line(
            "search",
            json!({ "service": "soulseek", "query": "inxs", "album": null }),
            1_795_000_000_000,
        );
        assert_eq!(value["ts"], 1_795_000_000_000_i64);
        assert_eq!(value["event"], "search");
        assert_eq!(value["query"], "inxs");
        assert!(value.get("album").is_none());
        let id = value["id"].as_str().unwrap();
        assert!(id.starts_with("1795000000000-"));
        assert_ne!(id, line("search", json!({}), 1_795_000_000_000)["id"]);
    }

    #[test]
    fn appends_one_line_per_activity_to_the_day_file() {
        let dir = temp_dir("append");
        let ts = now_ms();
        append(&dir, &line("browse", json!({ "username": "alice" }), ts)).unwrap();
        append(&dir, &line("browse", json!({ "username": "bob" }), ts)).unwrap();
        let day = Local.timestamp_millis_opt(ts).unwrap().format("%Y-%m-%d");
        let text = std::fs::read_to_string(dir.join(format!("activity.{day}.jsonl"))).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["username"], "bob");
        assert!(text.ends_with('\n'));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_removes_only_old_activity_files() {
        let dir = temp_dir("prune");
        std::fs::write(dir.join("activity.2020-01-01.jsonl"), "{}\n").unwrap();
        std::fs::write(dir.join("connection.2020-01-01.jsonl"), "{}\n").unwrap();
        let later = SystemTime::now() + Duration::from_secs((KEEP_DAYS + 1) * 24 * 60 * 60);
        prune(&dir, later);
        assert!(!dir.join("activity.2020-01-01.jsonl").exists());
        assert!(dir.join("connection.2020-01-01.jsonl").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
