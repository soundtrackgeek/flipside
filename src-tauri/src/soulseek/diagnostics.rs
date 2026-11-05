use serde::Serialize;
use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_RECENT_ENTRIES: usize = 120;

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub event: String,
    pub message: String,
}

#[derive(Clone)]
pub struct Diagnostics {
    path: PathBuf,
    entries: Arc<Mutex<VecDeque<DiagnosticEntry>>>,
}

impl Diagnostics {
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let diagnostics = Self {
            path,
            entries: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_RECENT_ENTRIES))),
        };
        diagnostics.record(
            "info",
            "diagnostics_ready",
            "Connection diagnostics started.",
        );
        Ok(diagnostics)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn recent(&self) -> Vec<DiagnosticEntry> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn record(&self, level: &str, event: &str, message: &str) {
        let entry = DiagnosticEntry {
            timestamp_ms: timestamp_ms(),
            level: sanitize(level, 16),
            event: sanitize(event, 64),
            message: sanitize(message, 500),
        };

        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if entries.len() == MAX_RECENT_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }

        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(
                file,
                "{}\t{}\t{}\t{}",
                entry.timestamp_ms, entry.level, entry.event, entry.message
            );
        }
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn sanitize(value: &str, max_length: usize) -> String {
    value
        .replace(['\r', '\n', '\t'], " ")
        .chars()
        .take(max_length)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_bounded_sanitized_diagnostics() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let diagnostics =
            Diagnostics::new(directory.path().join("connection.log")).expect("diagnostics");

        diagnostics.record("warn", "socket_lost", "Line one\nLine two");

        let entries = diagnostics.recent();
        assert_eq!(
            entries.last().expect("latest entry").message,
            "Line one Line two"
        );
        let file = std::fs::read_to_string(diagnostics.path()).expect("diagnostic log");
        assert!(file.contains("socket_lost"));
        assert!(!file.contains("Line one\nLine two"));
    }
}
