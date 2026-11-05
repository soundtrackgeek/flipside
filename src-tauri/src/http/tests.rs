use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Barrier;

fn ready(gate: &Gate, host: &str, now: Instant) -> Permit {
    match gate.admission(host, now) {
        Admission::Ready(permit) => permit,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn response(status: u16, headers: &[(&str, &str)]) -> ureq::Response {
    let headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    format!("HTTP/1.1 {status} Test\r\n{headers}Content-Length: 0\r\n\r\n")
        .parse()
        .unwrap()
}

#[test]
fn musicbrainz_uses_the_same_client_and_versioned_user_agent() {
    assert!(std::ptr::eq(musicbrainz(), client()));
    assert!(std::ptr::eq(agent(), &musicbrainz().agent));
    assert!(USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
    assert!(USER_AGENT.contains("https://github.com/soundtrackgeek/music_backup_v5"));
    assert!(BROWSER_USER_AGENT.ends_with(USER_AGENT));
    assert_eq!(canonical_host("WWW.MusicBrainz.org."), "musicbrainz.org");
}

#[test]
fn concurrent_musicbrainz_callers_share_one_token_without_bursts() {
    let gate = Arc::new(Gate::default());
    let now = Instant::now();
    for refill in [Duration::ZERO, Duration::from_millis(1100)] {
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let gate = gate.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    matches!(
                        gate.admission("musicbrainz.org", now + refill),
                        Admission::Ready(_)
                    )
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .filter(|ready| *ready)
                .count(),
            1
        );
    }
    assert!(matches!(
        gate.admission("musicbrainz.org", now + Duration::from_millis(2199)),
        Admission::Wait(_)
    ));
    // Another provider has an independent token even while MB is waiting.
    ready(&gate, "api.deezer.com", now);
}

#[test]
fn provider_rate_floors_are_preserved() {
    assert_eq!(
        host_interval("musicbrainz.org"),
        Duration::from_millis(1100)
    );
    assert_eq!(
        host_interval("api.discogs.com"),
        Duration::from_millis(1200)
    );
    assert_eq!(
        host_interval("ws.audioscrobbler.com"),
        Duration::from_millis(350)
    );
    assert_eq!(host_interval("api.deezer.com"), Duration::from_millis(200));
    assert_eq!(
        host_interval("coverartarchive.org"),
        Duration::from_millis(1200)
    );
    assert_eq!(
        host_interval("ia801.archive.org"),
        Duration::from_millis(1200)
    );
}

#[test]
fn retry_after_accepts_seconds_and_http_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784111777);
    assert_eq!(
        parse_retry_after(" 12 ", now),
        Some(Duration::from_secs(12))
    );
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:50:37 GMT", now),
        Some(Duration::from_secs(60))
    );
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:48:37 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(parse_retry_after("invalid", now), None);
}

#[test]
fn backoff_is_exponential_bounded_and_jittered() {
    for (failure, minimum) in [(1, 500), (2, 1000), (3, 2000), (4, 4000), (100, 4000)] {
        for seed in 0..100 {
            let delay = retry_delay(failure, seed);
            assert!(delay >= Duration::from_millis(minimum));
            assert!(delay <= Duration::from_millis(minimum + 250));
        }
    }
}

#[test]
fn retry_after_defers_all_callers_and_does_not_block_other_hosts() {
    let gate = Gate::default();
    let now = Instant::now();
    let permit = ready(&gate, "musicbrainz.org", now);
    gate.observe(
        "musicbrainz.org",
        permit,
        Some(&response(503, &[("Retry-After", "600")])),
        now,
        SystemTime::now(),
    );
    assert!(!gate.can_retry("musicbrainz.org", now));
    assert!(matches!(
        gate.admission("musicbrainz.org", now),
        Admission::Unavailable
    ));
    ready(&gate, "api.deezer.com", now);
    ready(&gate, "musicbrainz.org", now + Duration::from_secs(600));
}

#[test]
fn exhausted_discogs_budget_defers_and_limit_adjusts_spacing() {
    let gate = Gate::default();
    let now = Instant::now();
    let permit = ready(&gate, "api.discogs.com", now);
    gate.observe(
        "api.discogs.com",
        permit,
        Some(&response(
            200,
            &[
                ("X-Discogs-Ratelimit", "20"),
                ("X-Discogs-Ratelimit-Remaining", "0"),
            ],
        )),
        now,
        SystemTime::now(),
    );
    assert!(matches!(
        gate.admission("api.discogs.com", now),
        Admission::Unavailable
    ));
    assert_eq!(
        gate.hosts.lock().unwrap()["api.discogs.com"].interval,
        Duration::from_secs(3)
    );
    ready(&gate, "api.discogs.com", now + Duration::from_secs(60));
}

#[test]
fn circuit_recovers_with_one_probe_and_ignores_stale_successes() {
    let gate = Gate::default();
    let start = Instant::now();
    let mut old_permit = None;
    for failure in 0..3 {
        let now = start + Duration::from_secs(failure * 5);
        let permit = ready(&gate, "musicbrainz.org", now);
        old_permit = Some(permit);
        gate.observe(
            "musicbrainz.org",
            permit,
            Some(&response(503, &[])),
            now,
            SystemTime::now(),
        );
    }
    let opened = start + Duration::from_secs(10);
    gate.observe(
        "musicbrainz.org",
        old_permit.unwrap(),
        Some(&response(200, &[])),
        opened,
        SystemTime::now(),
    );
    assert!(matches!(
        gate.admission("musicbrainz.org", opened + Duration::from_secs(19)),
        Admission::Unavailable
    ));
    let probe_at = opened + CIRCUIT_COOLDOWN;
    let probe = ready(&gate, "musicbrainz.org", probe_at);
    assert!(matches!(
        gate.admission("musicbrainz.org", probe_at),
        Admission::Unavailable
    ));
    gate.observe(
        "musicbrainz.org",
        probe,
        Some(&response(200, &[])),
        probe_at,
        SystemTime::now(),
    );
    ready(
        &gate,
        "musicbrainz.org",
        probe_at + Duration::from_millis(1100),
    );
}

#[test]
fn failed_probe_reopens_circuit_and_transport_outages_are_counted() {
    let gate = Gate::default();
    let start = Instant::now();
    for failure in 0..3 {
        let now = start + Duration::from_secs(failure * 5);
        let permit = ready(&gate, "example.com", now);
        gate.observe("example.com", permit, None, now, SystemTime::now());
    }
    let probe_at = start + Duration::from_secs(30);
    let probe = ready(&gate, "example.com", probe_at);
    gate.observe(
        "example.com",
        probe,
        Some(&response(503, &[])),
        probe_at,
        SystemTime::now(),
    );
    assert!(matches!(
        gate.admission("example.com", probe_at + Duration::from_secs(19)),
        Admission::Unavailable
    ));
    ready(&gate, "example.com", probe_at + CIRCUIT_COOLDOWN);
}

#[test]
fn late_response_cannot_lose_a_longer_provider_cooldown() {
    let gate = Gate::default();
    let start = Instant::now();
    let mut old_permit = None;
    for failure in 0..3 {
        let now = start + Duration::from_secs(failure * 5);
        let permit = ready(&gate, "musicbrainz.org", now);
        old_permit = Some(permit);
        gate.observe(
            "musicbrainz.org",
            permit,
            Some(&response(503, &[])),
            now,
            SystemTime::now(),
        );
    }
    gate.observe(
        "musicbrainz.org",
        old_permit.unwrap(),
        Some(&response(503, &[("Retry-After", "600")])),
        start + Duration::from_secs(11),
        SystemTime::now(),
    );
    assert!(matches!(
        gate.admission("musicbrainz.org", start + Duration::from_secs(30)),
        Admission::Unavailable
    ));
    ready(&gate, "musicbrainz.org", start + Duration::from_secs(611));
}

#[test]
fn late_discogs_quota_does_not_close_circuit_or_lose_cooldown() {
    let gate = Gate::default();
    let start = Instant::now();
    let old_permit = ready(&gate, "api.discogs.com", start);
    for failure in 0..3 {
        let now = start + Duration::from_secs((failure + 1) * 5);
        let permit = ready(&gate, "api.discogs.com", now);
        gate.observe("api.discogs.com", permit, None, now, SystemTime::now());
    }
    gate.observe(
        "api.discogs.com",
        old_permit,
        Some(&response(200, &[("X-Discogs-Ratelimit-Remaining", "0")])),
        start + Duration::from_secs(16),
        SystemTime::now(),
    );
    assert!(matches!(
        gate.admission("api.discogs.com", start + Duration::from_secs(35)),
        Admission::Unavailable
    ));
    ready(&gate, "api.discogs.com", start + Duration::from_secs(76));
    assert!(matches!(
        gate.admission("api.discogs.com", start + Duration::from_secs(76)),
        Admission::Unavailable
    ));
}

#[test]
fn redirects_drop_cross_origin_credentials_and_preserve_artwork_headers() {
    for header in [
        "Authorization",
        "X-Api-Key",
        "X-Provider-Token",
        "Cookie",
        "Proxy-Authorization",
        "Host",
    ] {
        assert!(!redirect_header_allowed(header, false), "{header}");
    }
    for header in ["Accept", "Accept-Language", "Range", "If-None-Match"] {
        assert!(redirect_header_allowed(header, false), "{header}");
    }
    assert!(redirect_header_allowed("Authorization", true));
    assert!(!redirect_header_allowed("Cookie", true));
}

/// A small real HTTP transport verifies the wrapper, middleware, and ureq's
/// status conversion together. No external provider or wall-clock assumptions.
fn server(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut requests = Vec::new();
        for reply in replies {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "mock server did not receive expected request"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock accept: {error}"),
                }
            };
            // Windows may inherit the listener's nonblocking mode. Switch the
            // accepted connection back before reading a complete HTTP request.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0, "client closed before sending request headers");
                bytes.extend_from_slice(&buffer[..count]);
            };
            let body_length = String::from_utf8_lossy(&bytes[..header_end])
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, length)| length.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            // Consume POST bodies before closing, avoiding a TCP reset racing
            // the client's response read on Windows.
            while bytes.len() < header_end + body_length {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0, "client closed before sending request body");
                bytes.extend_from_slice(&buffer[..count]);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), thread)
}

fn wire_response(status: u16, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\nConnection: close\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn local_client() -> Client {
    Client::build(
        ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(Duration::from_secs(5)),
    )
}

#[test]
fn get_retries_429_and_503_and_preserves_queries_headers_and_user_agent() {
    let (url, server) = server(vec![
        wire_response(429, "Retry-After: 0\r\n", "busy"),
        wire_response(503, "", "busy"),
        wire_response(200, "", "ok"),
    ]);
    let client = local_client();
    let response = client
        .get(&url)
        .query("query", "AC/DC & Bowie")
        .set("Accept", "application/json")
        .set("User-Agent", "stale-version")
        .call()
        .unwrap();
    assert_eq!(response.into_string().unwrap(), "ok");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    for request in requests {
        assert!(request.contains("query=AC%2FDC+%26+Bowie"));
        assert!(request.contains(USER_AGENT));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept: application/json"));
        assert!(!request.contains("stale-version"));
    }
}

#[test]
fn permanent_status_and_body_are_returned_without_retry() {
    let (url, server) = server(vec![wire_response(404, "", "missing")]);
    match local_client().get(&url).call() {
        Err(ureq::Error::Status(404, response)) => {
            assert_eq!(response.into_string().unwrap(), "missing")
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn long_retry_after_returns_original_status_and_following_calls_fail_fast() {
    let (url, server) = server(vec![wire_response(
        503,
        "Retry-After: 600\r\n",
        "maintenance",
    )]);
    let client = local_client();
    match client.get(&url).call() {
        Err(ureq::Error::Status(503, response)) => {
            assert_eq!(response.into_string().unwrap(), "maintenance")
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(matches!(
        client.get(&url).call(),
        Err(ureq::Error::Transport(_))
    ));
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn raw_post_uses_shared_policy_without_replaying_503() {
    let (url, server) = server(vec![wire_response(503, "", "busy")]);
    let client = local_client();
    assert!(matches!(
        client.agent.post(&url).send_string(""),
        Err(ureq::Error::Status(503, _))
    ));
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST "));
    assert!(requests[0].contains(USER_AGENT));
    assert_eq!(
        client.gate.hosts.lock().unwrap()[&gate_key(&url::Url::parse(&url).unwrap())].failures,
        1
    );
}

#[test]
fn artwork_redirects_follow_and_strip_custom_tokens_on_different_ports() {
    let (destination, target) = server(vec![wire_response(200, "", "art")]);
    let (url, source) = server(vec![wire_response(
        302,
        &format!("Location: {destination}/image\r\n"),
        "",
    )]);
    let response = local_client()
        .get(&url)
        .set("X-Api-Key", "private")
        .set("Accept", "image/*")
        .call()
        .unwrap();
    assert_eq!(response.into_string().unwrap(), "art");
    assert!(source.join().unwrap()[0].contains("private"));
    let forwarded = target.join().unwrap();
    assert!(!forwarded[0].contains("private"));
    assert!(forwarded[0]
        .to_ascii_lowercase()
        .contains("accept: image/*"));
}

#[test]
fn no_retry_get_still_follows_redirects_and_preserves_failure() {
    let (destination, target) = server(vec![wire_response(503, "", "busy")]);
    let (url, source) = server(vec![wire_response(
        302,
        &format!("Location: {destination}/download\r\n"),
        "",
    )]);
    let response = local_client().get(&url).without_retries().call();
    assert!(matches!(response, Err(ureq::Error::Status(503, _))));
    assert_eq!(source.join().unwrap().len(), 1);
    assert_eq!(target.join().unwrap().len(), 1);
}

#[test]
fn expired_request_deadline_returns_without_consuming_a_token() {
    let gate = Gate::default();
    let now = Instant::now();
    let _deadline = DeadlineGuard::new(Some(now));
    assert!(matches!(
        gate.acquire("musicbrainz.org"),
        Err(ureq::Error::Transport(_))
    ));
    assert!(gate.hosts.lock().unwrap().is_empty());
}

#[test]
fn nested_deadline_context_is_restored() {
    let deadline = Instant::now() + Duration::from_secs(1);
    assert!(REQUEST_DEADLINE.get().is_none());
    {
        let _outer = DeadlineGuard::new(Some(deadline));
        {
            let _inner = DeadlineGuard::new(Some(deadline + Duration::from_secs(1)));
            assert_eq!(
                REQUEST_DEADLINE.get(),
                Some(deadline + Duration::from_secs(1))
            );
        }
        assert_eq!(REQUEST_DEADLINE.get(), Some(deadline));
    }
    assert!(REQUEST_DEADLINE.get().is_none());
}

#[test]
fn local_services_on_different_ports_have_independent_circuits() {
    let (outage_url, outage) = server(vec![wire_response(503, "Retry-After: 600\r\n", "busy")]);
    let (healthy_url, healthy) = server(vec![wire_response(200, "", "ok")]);
    let client = local_client();
    assert!(matches!(
        client.get(&outage_url).call(),
        Err(ureq::Error::Status(503, _))
    ));
    assert_eq!(
        client
            .get(&healthy_url)
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
        "ok"
    );
    assert_eq!(outage.join().unwrap().len(), 1);
    assert_eq!(healthy.join().unwrap().len(), 1);
    assert_eq!(
        gate_key(&url::Url::parse("https://www.musicbrainz.org:443/ws/2").unwrap()),
        "musicbrainz.org"
    );
}
