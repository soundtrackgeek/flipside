//! Shared provider transport. Every attempt (including redirected GETs and raw
//! POSTs) passes the same per-host gate; only GETs are automatically retried.

use std::cell::Cell;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub const USER_AGENT: &str = concat!(
    "MusicLibrary/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/soundtrackgeek/music_backup_v5 )"
);

pub const BROWSER_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 MusicLibrary/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/soundtrackgeek/music_backup_v5 )"
);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GATE_WAIT: Duration = Duration::from_secs(10);
const MAX_RETRY_WAIT: Duration = Duration::from_secs(5);
const CIRCUIT_COOLDOWN: Duration = Duration::from_secs(20);
const MAX_ATTEMPTS: usize = 3;
const MAX_REDIRECTS: usize = 5;

static CLIENT: OnceLock<Client> = OnceLock::new();

thread_local! {
    // ureq 2's middleware has no timeout getter. This scoped context passes a
    // synchronous wrapper's overall deadline into the shared rate gate without
    // sending internal headers or relying on transport-private fields.
    static REQUEST_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

struct DeadlineGuard(Option<Instant>);

impl DeadlineGuard {
    fn new(deadline: Option<Instant>) -> Self {
        Self(REQUEST_DEADLINE.replace(deadline))
    }
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        REQUEST_DEADLINE.set(self.0);
    }
}

/// Reuses the provider connection pool and gate without replaying the request.
/// Raw requests deliberately do not follow redirects. Authenticated sessions
/// needing their own cookie jar must retain their own agent.
pub fn agent() -> &'static ureq::Agent {
    &client().agent
}

pub fn get(url: &str) -> Request<'static> {
    client().get(url)
}

/// This is the same client and gate used by generic GETs, not a separate pool.
pub fn musicbrainz() -> &'static Client {
    client()
}

fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::build(
            ureq::AgentBuilder::new()
                .timeout(REQUEST_TIMEOUT)
                .timeout_connect(Duration::from_secs(10))
                .try_proxy_from_env(true),
        )
    })
}

pub struct Client {
    agent: ureq::Agent,
    gate: Arc<Gate>,
}

impl Client {
    fn build(builder: ureq::AgentBuilder) -> Self {
        let gate = Arc::new(Gate::default());
        let agent = builder
            .user_agent(USER_AGENT)
            // Automatic redirects bypass middleware. Follow GET redirects here
            // so each destination, including MusicBrainz, acquires a token.
            .redirects(0)
            .middleware(ProviderPolicy(gate.clone()))
            .build();
        Self { agent, gate }
    }

    pub fn get(&self, url: &str) -> Request<'_> {
        Request {
            client: self,
            inner: self.agent.get(url),
            timeout: REQUEST_TIMEOUT,
            max_attempts: MAX_ATTEMPTS,
        }
    }
}

#[must_use = "Requests do nothing until call()"]
pub struct Request<'a> {
    client: &'a Client,
    inner: ureq::Request,
    timeout: Duration,
    max_attempts: usize,
}

impl Request<'_> {
    pub fn set(mut self, header: &str, value: &str) -> Self {
        self.inner = self.inner.set(header, value);
        self
    }

    pub fn query(mut self, name: &str, value: &str) -> Self {
        self.inner = self.inner.query(name, value);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Follow safe GET redirects, but never replay a failed response. Useful
    /// for download endpoints whose GET may consume provider-side quota.
    pub fn without_retries(mut self) -> Self {
        self.max_attempts = 1;
        self
    }

    pub fn call(self) -> Result<ureq::Response, ureq::Error> {
        let started = Instant::now();
        let _deadline = DeadlineGuard::new(started.checked_add(self.timeout));
        let mut request = self.inner;
        let mut attempts = 0;
        let mut redirects = 0;
        loop {
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(transport_error("Provider request timed out"));
            }
            let result = request.clone().timeout(remaining).call();
            match result {
                Err(ureq::Error::Status(status @ (429 | 503), response)) => {
                    attempts += 1;
                    let url = request.request_url()?;
                    // Return the original status/body when retrying would wait
                    // too long or when the circuit opened. Callers can still
                    // distinguish a provider 429/503 from a transport failure.
                    if attempts >= self.max_attempts
                        || !self
                            .client
                            .gate
                            .can_retry(&gate_key(url.as_url()), Instant::now())
                        || started.elapsed() >= self.timeout
                    {
                        return Err(ureq::Error::Status(status, response));
                    }
                    // Dropping a failed response rather than draining an
                    // unbounded body also bounds time spent retrying outages.
                }
                Ok(response) if matches!(response.status(), 301 | 302 | 303 | 307 | 308) => {
                    let Some(location) = response.header("Location") else {
                        return Ok(response);
                    };
                    if redirects >= MAX_REDIRECTS {
                        return Err(transport_error("Too many provider redirects"));
                    }
                    let current = request.request_url()?.as_url().clone();
                    let next = current.join(location)?;
                    if !matches!(next.scheme(), "http" | "https")
                        || (current.scheme() == "https" && next.scheme() != "https")
                        || !next.username().is_empty()
                        || next.password().is_some()
                    {
                        return Err(transport_error("Unsafe provider redirect"));
                    }
                    let same_origin = current.origin() == next.origin();
                    let mut redirected = self.client.agent.get(next.as_str());
                    for header in request.header_names() {
                        if redirect_header_allowed(&header, same_origin) {
                            if let Some(value) = request.header(&header) {
                                redirected = redirected.set(&header, value);
                            }
                        }
                    }
                    request = redirected;
                    redirects += 1;
                }
                other => return other,
            }
        }
    }
}

fn redirect_header_allowed(header: &str, same_origin: bool) -> bool {
    let header = header.to_ascii_lowercase();
    if matches!(header.as_str(), "host" | "cookie" | "proxy-authorization") {
        return false;
    }
    same_origin
        || matches!(
            header.as_str(),
            "accept"
                | "accept-language"
                | "accept-encoding"
                | "range"
                | "if-range"
                | "if-none-match"
                | "if-modified-since"
        )
}

struct ProviderPolicy(Arc<Gate>);

impl ureq::Middleware for ProviderPolicy {
    fn handle(
        &self,
        request: ureq::Request,
        next: ureq::MiddlewareNext,
    ) -> Result<ureq::Response, ureq::Error> {
        let host = gate_key(request.request_url()?.as_url());
        let permit = self.0.acquire(&host)?;
        let result = next.handle(request.set("User-Agent", USER_AGENT));
        let response = match &result {
            Ok(response) | Err(ureq::Error::Status(_, response)) => Some(response),
            Err(ureq::Error::Transport(_)) => None,
        };
        self.0
            .observe(&host, permit, response, Instant::now(), SystemTime::now());
        result
    }
}

#[derive(Default)]
struct Gate {
    hosts: Mutex<HashMap<String, HostState>>,
    jitter: AtomicU64,
}

struct HostState {
    interval: Duration,
    next_start: Instant,
    deferred_until: Instant,
    failures: u32,
    circuit_until: Option<Instant>,
    probe_in_flight: bool,
    generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct Permit {
    generation: u64,
}

#[derive(Debug)]
enum Admission {
    Ready(Permit),
    Wait(Duration),
    Unavailable,
}

impl HostState {
    fn new(host: &str, now: Instant) -> Self {
        Self {
            interval: host_interval(host),
            next_start: now,
            deferred_until: now,
            failures: 0,
            circuit_until: None,
            probe_in_flight: false,
            generation: 0,
        }
    }

    fn unavailable(&self, now: Instant) -> bool {
        self.probe_in_flight
            || self.circuit_until.is_some_and(|until| now < until)
            || self.deferred_until.saturating_duration_since(now) > MAX_RETRY_WAIT
    }
}

impl Gate {
    fn acquire(&self, host: &str) -> Result<Permit, ureq::Error> {
        let started = Instant::now();
        let deadline = REQUEST_DEADLINE
            .get()
            .unwrap_or(started + MAX_GATE_WAIT)
            .min(started + MAX_GATE_WAIT);
        loop {
            let now = Instant::now();
            let budget = deadline.saturating_duration_since(now);
            if budget.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Provider request timed out waiting for the rate limit",
                )
                .into());
            }
            match self.admission(host, now) {
                Admission::Ready(permit) => return Ok(permit),
                Admission::Unavailable => {
                    return Err(transport_error(
                        "Provider temporarily unavailable; try again later",
                    ));
                }
                Admission::Wait(wait) => {
                    // Recheck shared state promptly if another request opens
                    // the circuit or receives a longer Retry-After.
                    std::thread::sleep(wait.min(budget).min(Duration::from_millis(100)));
                }
            }
        }
    }

    // A one-token bucket: only this mutex-protected transition can consume the
    // current token; it refills after interval and never accumulates a burst.
    fn admission(&self, host: &str, now: Instant) -> Admission {
        let mut hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        let state = hosts
            .entry(host.to_owned())
            .or_insert_with(|| HostState::new(host, now));
        if state.unavailable(now) {
            return Admission::Unavailable;
        }
        let ready_at = state.next_start.max(state.deferred_until);
        if now < ready_at {
            return Admission::Wait(ready_at.duration_since(now));
        }
        if state.circuit_until.is_some() {
            state.probe_in_flight = true;
        }
        state.next_start = now + state.interval;
        Admission::Ready(Permit {
            generation: state.generation,
        })
    }

    fn can_retry(&self, host: &str, now: Instant) -> bool {
        self.hosts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(host)
            .is_none_or(|state| !state.unavailable(now))
    }

    fn observe(
        &self,
        host: &str,
        permit: Permit,
        response: Option<&ureq::Response>,
        now: Instant,
        wall_time: SystemTime,
    ) {
        let mut hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        let Some(state) = hosts.get_mut(host) else {
            return;
        };
        let retry_after = response
            .filter(|response| matches!(response.status(), 429 | 503))
            .and_then(|response| response.header("Retry-After"))
            .and_then(|value| parse_retry_after(value, wall_time))
            .unwrap_or_default();
        // A late response still carries a valid provider-wide cooldown, even
        // when its request predates the currently open circuit.
        if let Some(until) = now.checked_add(retry_after) {
            state.deferred_until = state.deferred_until.max(until);
        }
        if host == "api.discogs.com" {
            if let Some(response) = response {
                apply_discogs_headers(state, response, now);
            }
        }
        // An older in-flight success must not close a circuit that a newer
        // failure opened. Its quota and cooldown headers still apply above.
        if permit.generation != state.generation {
            return;
        }
        let failed = response.is_none_or(|response| matches!(response.status(), 429 | 503));
        if failed {
            state.failures = state.failures.saturating_add(1);
            let jitter = self.jitter.fetch_add(1, Ordering::Relaxed);
            let delay = retry_delay(state.failures, jitter);
            // Very large server delays are retained but never slept through.
            let until = now
                .checked_add(delay.max(retry_after))
                .unwrap_or(now + Duration::from_secs(86400));
            state.deferred_until = state.deferred_until.max(until);
            if state.failures >= MAX_ATTEMPTS as u32 || state.probe_in_flight {
                state.circuit_until = Some(now + CIRCUIT_COOLDOWN);
                state.probe_in_flight = false;
                state.generation = state.generation.wrapping_add(1);
            }
        } else {
            state.failures = 0;
            state.circuit_until = None;
            state.probe_in_flight = false;
        }
    }
}

fn host_interval(host: &str) -> Duration {
    match host.trim_end_matches('.') {
        "musicbrainz.org" | "www.musicbrainz.org" => Duration::from_millis(1100),
        "api.discogs.com" | "coverartarchive.org" | "i.discogs.com" => Duration::from_millis(1200),
        "ws.audioscrobbler.com" => Duration::from_millis(350),
        "api.listenbrainz.org" => Duration::from_millis(500),
        "api.deezer.com" => Duration::from_millis(200),
        "archive.org" => Duration::from_millis(1200),
        host if host.ends_with(".archive.org") => Duration::from_millis(1200),
        _ => Duration::from_millis(100),
    }
}

fn canonical_host(host: &str) -> String {
    match host.trim_end_matches('.').to_ascii_lowercase().as_str() {
        "www.musicbrainz.org" => "musicbrainz.org".to_owned(),
        host => host.to_owned(),
    }
}

fn gate_key(url: &url::Url) -> String {
    let host = canonical_host(url.host_str().unwrap_or_default());
    // Configurable local services often share a machine with different ports;
    // one must not open the other's circuit. URL normalizes default ports.
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn retry_delay(failures: u32, sequence: u64) -> Duration {
    // SplitMix64 mixing supplies bounded jitter without another dependency or
    // a globally locked RNG. Wall time de-synchronizes different app processes.
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let mut mixed = sequence.wrapping_add(seed).wrapping_add(0x9e3779b97f4a7c15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
    mixed ^= mixed >> 31;
    Duration::from_millis((500_u64 << failures.saturating_sub(1).min(3)) + mixed % 251)
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let now: chrono::DateTime<chrono::Utc> = now.into();
    Some(date.signed_duration_since(now).to_std().unwrap_or_default())
}

fn apply_discogs_headers(state: &mut HostState, response: &ureq::Response, now: Instant) {
    if let Some(limit) = response
        .header("X-Discogs-Ratelimit")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&n| n > 0)
    {
        state.interval =
            host_interval("api.discogs.com").max(Duration::from_secs_f64(60.0 / limit as f64));
        state.next_start = state.next_start.max(now + state.interval);
    }
    if response
        .header("X-Discogs-Ratelimit-Remaining")
        .and_then(|s| s.parse::<u32>().ok())
        == Some(0)
    {
        // Discogs exposes no reset time. Defer a full rolling minute instead
        // of immediately issuing another request against an exhausted budget.
        state.deferred_until = state.deferred_until.max(now + Duration::from_secs(60));
    }
}

fn transport_error(message: &str) -> ureq::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message).into()
}

#[cfg(test)]
mod tests;
