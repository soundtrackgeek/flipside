use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

const DISTRIBUTED_EVENT: &str = "flipside://soulseek-distributed-status";
const COUNTER_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);
const REQUEST_RATE_WINDOW: Duration = Duration::from_secs(1);
const MAX_REQUESTS_PER_WINDOW: usize = 300;
const MAX_REQUESTS_PER_USER_WINDOW: usize = 12;
const DUPLICATE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_SEEN_REQUESTS: usize = 8_192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DistributedState {
    Offline,
    Discovering,
    Connected,
    BranchRoot,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DistributedSnapshot {
    pub state: DistributedState,
    pub message: String,
    pub branch_level: Option<u32>,
    pub searches_received: u64,
    pub searches_matched: u64,
    pub searches_answered: u64,
    pub searches_ignored: u64,
    pub updated_at_ms: u64,
}

impl DistributedSnapshot {
    fn offline() -> Self {
        Self {
            state: DistributedState::Offline,
            message: "Connect to Soulseek to join global search.".to_owned(),
            branch_level: None,
            searches_received: 0,
            searches_matched: 0,
            searches_answered: 0,
            searches_ignored: 0,
            updated_at_ms: timestamp_ms(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAdmission {
    Allowed,
    Duplicate,
    RateLimited,
}

struct RequestGuard {
    recent: VecDeque<Instant>,
    recent_by_user: HashMap<String, VecDeque<Instant>>,
    seen: HashMap<(String, u32), Instant>,
    seen_order: VecDeque<((String, u32), Instant)>,
}

impl RequestGuard {
    fn new() -> Self {
        Self {
            recent: VecDeque::new(),
            recent_by_user: HashMap::new(),
            seen: HashMap::new(),
            seen_order: VecDeque::new(),
        }
    }

    fn admit(&mut self, username: &str, token: u32, now: Instant) -> RequestAdmission {
        prune_times(&mut self.recent, now, REQUEST_RATE_WINDOW);
        self.prune_seen(now);

        let username = username.to_ascii_lowercase();
        let key = (username.clone(), token);
        if self.seen.contains_key(&key) {
            return RequestAdmission::Duplicate;
        }

        let user_recent = self.recent_by_user.entry(username).or_default();
        prune_times(user_recent, now, REQUEST_RATE_WINDOW);
        if self.recent.len() >= MAX_REQUESTS_PER_WINDOW
            || user_recent.len() >= MAX_REQUESTS_PER_USER_WINDOW
        {
            return RequestAdmission::RateLimited;
        }

        self.recent.push_back(now);
        user_recent.push_back(now);
        self.seen.insert(key.clone(), now);
        self.seen_order.push_back((key, now));
        RequestAdmission::Allowed
    }

    fn prune_seen(&mut self, now: Instant) {
        while self.seen_order.front().is_some_and(|(_, seen_at)| {
            now.duration_since(*seen_at) > DUPLICATE_TTL || self.seen.len() >= MAX_SEEN_REQUESTS
        }) {
            let Some((key, seen_at)) = self.seen_order.pop_front() else {
                break;
            };
            if self
                .seen
                .get(&key)
                .is_some_and(|current| *current == seen_at)
            {
                self.seen.remove(&key);
            }
        }
    }
}

struct DistributedInner {
    snapshot: DistributedSnapshot,
    guard: RequestGuard,
    last_counter_publish: Instant,
}

#[derive(Clone)]
pub struct DistributedHub {
    app: AppHandle,
    inner: Arc<Mutex<DistributedInner>>,
}

impl DistributedHub {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            inner: Arc::new(Mutex::new(DistributedInner {
                snapshot: DistributedSnapshot::offline(),
                guard: RequestGuard::new(),
                last_counter_publish: Instant::now(),
            })),
        }
    }

    pub fn snapshot(&self) -> DistributedSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot
            .clone()
    }

    pub fn begin_discovery(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.snapshot = DistributedSnapshot {
            state: DistributedState::Discovering,
            message: "Finding a global-search relay…".to_owned(),
            branch_level: None,
            searches_received: 0,
            searches_matched: 0,
            searches_answered: 0,
            searches_ignored: 0,
            updated_at_ms: timestamp_ms(),
        };
        inner.guard = RequestGuard::new();
        inner.last_counter_publish = Instant::now();
        let snapshot = inner.snapshot.clone();
        drop(inner);
        self.publish(snapshot);
    }

    pub fn connected(&self, branch_level: u32) {
        self.set_status(
            DistributedState::Connected,
            "Global search connected.",
            Some(branch_level),
        );
    }

    pub fn branch_root(&self) {
        self.set_status(
            DistributedState::BranchRoot,
            "Global search connected as a branch root.",
            Some(0),
        );
    }

    pub fn rediscovering(&self) {
        self.set_status(
            DistributedState::Discovering,
            "Global-search relay lost; finding another…",
            None,
        );
    }

    pub fn offline(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut snapshot = DistributedSnapshot::offline();
        snapshot.searches_received = inner.snapshot.searches_received;
        snapshot.searches_matched = inner.snapshot.searches_matched;
        snapshot.searches_answered = inner.snapshot.searches_answered;
        snapshot.searches_ignored = inner.snapshot.searches_ignored;
        inner.snapshot = snapshot.clone();
        drop(inner);
        self.publish(snapshot);
    }

    pub fn admit_request(&self, username: &str, token: u32) -> RequestAdmission {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.snapshot.searches_received = inner.snapshot.searches_received.saturating_add(1);
        let admission = inner.guard.admit(username, token, now);
        if admission != RequestAdmission::Allowed {
            inner.snapshot.searches_ignored = inner.snapshot.searches_ignored.saturating_add(1);
        }
        let snapshot = counter_snapshot_if_due(&mut inner, now);
        drop(inner);
        if let Some(snapshot) = snapshot {
            self.publish(snapshot);
        }
        admission
    }

    pub fn record_match(&self) {
        self.update_counter(|snapshot| {
            snapshot.searches_matched = snapshot.searches_matched.saturating_add(1);
        });
    }

    pub fn record_answered(&self) {
        self.update_counter(|snapshot| {
            snapshot.searches_answered = snapshot.searches_answered.saturating_add(1);
        });
    }

    fn set_status(
        &self,
        state: DistributedState,
        message: impl Into<String>,
        branch_level: Option<u32>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.snapshot.state = state;
        inner.snapshot.message = message.into();
        inner.snapshot.branch_level = branch_level;
        inner.snapshot.updated_at_ms = timestamp_ms();
        let snapshot = inner.snapshot.clone();
        drop(inner);
        self.publish(snapshot);
    }

    fn update_counter(&self, update: impl FnOnce(&mut DistributedSnapshot)) {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut inner.snapshot);
        let snapshot = counter_snapshot_if_due(&mut inner, now);
        drop(inner);
        if let Some(snapshot) = snapshot {
            self.publish(snapshot);
        }
    }

    fn publish(&self, snapshot: DistributedSnapshot) {
        let _ = self.app.emit(DISTRIBUTED_EVENT, snapshot);
    }
}

fn counter_snapshot_if_due(
    inner: &mut DistributedInner,
    now: Instant,
) -> Option<DistributedSnapshot> {
    if now.duration_since(inner.last_counter_publish) < COUNTER_PUBLISH_INTERVAL {
        return None;
    }
    inner.last_counter_publish = now;
    inner.snapshot.updated_at_ms = timestamp_ms();
    Some(inner.snapshot.clone())
}

fn prune_times(times: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while times
        .front()
        .is_some_and(|timestamp| now.duration_since(*timestamp) > window)
    {
        times.pop_front();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_requests_are_rejected_without_consuming_rate_capacity() {
        let mut guard = RequestGuard::new();
        let now = Instant::now();
        assert_eq!(guard.admit("listener", 41, now), RequestAdmission::Allowed);
        assert_eq!(
            guard.admit("LISTENER", 41, now),
            RequestAdmission::Duplicate
        );
        assert_eq!(guard.recent.len(), 1);
    }

    #[test]
    fn per_user_search_bursts_are_bounded() {
        let mut guard = RequestGuard::new();
        let now = Instant::now();
        for token in 0..MAX_REQUESTS_PER_USER_WINDOW {
            assert_eq!(
                guard.admit("noisy-listener", token as u32, now),
                RequestAdmission::Allowed
            );
        }
        assert_eq!(
            guard.admit("noisy-listener", 999, now),
            RequestAdmission::RateLimited
        );
        assert_eq!(
            guard.admit("another-listener", 999, now),
            RequestAdmission::Allowed
        );
    }
}
