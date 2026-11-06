//! Local bridge for the Flipside browser extension.
//!
//! A tiny HTTP/1.1 server on `127.0.0.1` lets the Edge/Chrome extension ask "can Flipside get this album?"
//! (`POST /check`) and "get it" (`POST /get`). Every request must carry the pairing token as a bearer token,
//! a loopback `Host` header, and (when present) an extension `Origin`, so ordinary web pages cannot drive it.
//! `/get` does not download anything itself: it emits an event and the UI runs the same Auto-get flow as the
//! Auto-get button.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::catalog::{self, AlbumCard};

pub const PORT: u16 = 47615;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024;

fn token_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_config_dir().context("no config directory")?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("extension-token.txt"))
}

/// Random 256-bit hex token. `RandomState` is seeded by the OS CSPRNG on every construction.
fn generate_token() -> String {
    (0..4)
        .map(|_| {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default(),
            );
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// The pairing token, created on first use and kept in the app config folder.
pub fn token(app: &AppHandle) -> Result<String> {
    let path = token_path(app)?;
    if let Ok(text) = std::fs::read_to_string(&path) {
        let text = text.trim();
        if text.len() >= 32 {
            return Ok(text.to_owned());
        }
    }
    let fresh = generate_token();
    std::fs::write(&path, &fresh).context("Could not save the extension token")?;
    Ok(fresh)
}

/// Replaces the token, which unpairs every extension that still has the old one.
pub fn regenerate_token(app: &AppHandle) -> Result<String> {
    let fresh = generate_token();
    std::fs::write(token_path(app)?, &fresh).context("Could not save the extension token")?;
    Ok(fresh)
}

pub fn start(app: &AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Extension bridge could not listen on 127.0.0.1:{PORT}: {error}");
            return;
        }
    };
    let app = app.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = handle(&app, stream);
            });
        }
    });
}

struct Request {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            anyhow::bail!("headers too large");
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            anyhow::bail!("connection closed");
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default().to_ascii_uppercase();
    let path = first.next().unwrap_or("/").split('?').next().unwrap_or("/").to_owned();
    let (mut host, mut origin, mut authorization, mut length) = (String::new(), None, None, 0usize);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => host = value.to_ascii_lowercase(),
            "origin" => origin = Some(value.to_owned()),
            "authorization" => authorization = Some(value.to_owned()),
            "content-length" => length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    if length > MAX_BODY_BYTES {
        anyhow::bail!("body too large");
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(Request { method, path, host, origin, authorization, body })
}

fn extension_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://")
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn respond(stream: &mut TcpStream, status: &str, origin: Option<&str>, body: &Value) -> Result<()> {
    let payload = if status.starts_with("204") { String::new() } else { body.to_string() };
    let cors = origin
        .filter(|o| extension_origin(o))
        .map(|o| {
            format!(
                "Access-Control-Allow-Origin: {o}\r\nAccess-Control-Allow-Headers: authorization, content-type\r\n\
                 Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Max-Age: 600\r\nVary: Origin\r\n"
            )
        })
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cors}Cache-Control: no-store\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )?;
    stream.flush()?;
    Ok(())
}

#[derive(Deserialize)]
struct AlbumQuery {
    artist: String,
    album: String,
    #[serde(default)]
    year: Option<i32>,
}

fn handle(app: &AppHandle, mut stream: TcpStream) -> Result<()> {
    let request = read_request(&mut stream)?;
    let origin = request.origin.as_deref();

    let loopback_host = request.host == format!("127.0.0.1:{PORT}") || request.host == format!("localhost:{PORT}");
    if !loopback_host {
        return respond(&mut stream, "403 Forbidden", None, &json!({ "error": "bad host" }));
    }
    if let Some(origin) = origin {
        if !extension_origin(origin) {
            return respond(&mut stream, "403 Forbidden", None, &json!({ "error": "origin not allowed" }));
        }
    }
    if request.method == "OPTIONS" {
        return respond(&mut stream, "204 No Content", origin, &Value::Null);
    }

    let expected = token(app)?;
    let paired = request
        .authorization
        .as_deref()
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|given| constant_time_eq(given.trim(), &expected));

    if request.method == "GET" && request.path == "/ping" {
        return respond(
            &mut stream,
            "200 OK",
            origin,
            &json!({ "app": "flipside", "version": env!("CARGO_PKG_VERSION"), "paired": paired }),
        );
    }
    if !paired {
        return respond(&mut stream, "401 Unauthorized", origin, &json!({ "error": "not paired: paste the token from Flipside → Settings → Browser extension" }));
    }
    if request.method != "POST" {
        return respond(&mut stream, "405 Method Not Allowed", origin, &json!({ "error": "use POST" }));
    }
    let query: AlbumQuery = match serde_json::from_slice(&request.body) {
        Ok(query) => query,
        Err(error) => return respond(&mut stream, "400 Bad Request", origin, &json!({ "error": format!("bad body: {error}") })),
    };
    if query.artist.trim().is_empty() || query.album.trim().is_empty() {
        return respond(&mut stream, "400 Bad Request", origin, &json!({ "error": "artist and album are required" }));
    }

    let result = match request.path.as_str() {
        "/check" => check(app, &query),
        "/get" => get(app, &query),
        _ => return respond(&mut stream, "404 Not Found", origin, &json!({ "error": "unknown route" })),
    };
    match result {
        Ok(value) => respond(&mut stream, "200 OK", origin, &value),
        Err(error) => respond(&mut stream, "500 Internal Server Error", origin, &json!({ "error": format!("{error:#}") })),
    }
}

/// Drops edition noise like "(Deluxe Edition)" so Usenet and Deezer searches match the plain album.
fn simplify_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut bracket = String::new();
    for ch in title.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                bracket.clear();
                bracket.push(ch);
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                bracket.push(ch);
                if depth == 0 {
                    let lower = bracket.to_lowercase();
                    let noise = ["deluxe", "edition", "remaster", "version", "expanded", "anniversary", "bonus"];
                    if !noise.iter().any(|n| lower.contains(n)) {
                        out.push_str(&bracket);
                    }
                }
            }
            _ if depth > 0 => bracket.push(ch),
            _ => out.push(ch),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tries the title as written, then without edition noise ("OK Computer (Deluxe Edition)" -> "OK Computer").
fn resolve(artist: &str, album: &str) -> Result<Option<AlbumCard>> {
    if let Some(card) = catalog::resolve_album(artist, album)? {
        return Ok(Some(card));
    }
    let plain = simplify_title(album);
    if !plain.is_empty() && plain != album {
        return catalog::resolve_album(artist, &plain);
    }
    Ok(None)
}

fn check(app: &AppHandle, query: &AlbumQuery) -> Result<Value> {
    let artist = query.artist.trim();
    let album = query.album.trim();

    // Deezer: the album has to exist there and a Deezer login has to be saved.
    let deezer = match resolve(artist, album) {
        Ok(Some(card)) => {
            let arl = crate::deemix::credential_status().map(|s| s.configured).unwrap_or(false);
            json!({
                "state": if arl { "found" } else { "needs-login" },
                "detail": if arl { format!("{} - {}", card.artist, card.title) } else { "On Deezer, but no Deezer ARL is saved".to_owned() },
                "id": card.id, "cover": card.cover, "tracks": card.track_count, "year": card.year,
            })
        }
        Ok(None) => json!({ "state": "missing", "detail": "Not on Deezer" }),
        Err(error) => json!({ "state": "error", "detail": format!("{error:#}") }),
    };

    // Usenet: the best Prowlarr hit must clear the same confidence bar Auto-get uses.
    let usenet = match app.try_state::<crate::usenet::UsenetManager>() {
        None => json!({ "state": "error", "detail": "Usenet is not ready yet" }),
        Some(manager) => match manager.search(crate::usenet::UsenetSearchRequest {
            title: simplify_title(album),
            artist: artist.to_owned(),
            year: query.year,
            limit: Some(40),
        }) {
            Ok(response) => {
                let best = response.results.iter().map(|r| r.match_score).max().unwrap_or(0);
                if best >= 60 {
                    json!({ "state": "found", "detail": format!("{} releases, best match {best}%", response.results.len()) })
                } else if response.results.is_empty() {
                    json!({ "state": "missing", "detail": "No NZBs found" })
                } else {
                    json!({ "state": "missing", "detail": "No confident match" })
                }
            }
            Err(error) => json!({ "state": "unavailable", "detail": format!("{error:#}") }),
        },
    };

    // Soulseek: a real search takes ~20 seconds, so this only reports whether the network is reachable.
    let soulseek = match app.try_state::<crate::soulseek::BridgeConnectionManager>() {
        None => json!({ "state": "unavailable", "detail": "Soulseek is not ready yet" }),
        Some(manager) => {
            let snapshot = manager.current_snapshot();
            if matches!(snapshot.state, crate::soulseek::BridgeConnectionState::Online) {
                json!({ "state": "online", "detail": "Connected. Auto-get will search the network" })
            } else {
                json!({ "state": "unavailable", "detail": snapshot.message })
            }
        }
    };

    Ok(json!({ "artist": artist, "album": album, "deezer": deezer, "usenet": usenet, "soulseek": soulseek }))
}

fn get(app: &AppHandle, query: &AlbumQuery) -> Result<Value> {
    let artist = query.artist.trim();
    let album = query.album.trim();
    // Prefer the Deezer card (cover, track count, id); otherwise Auto-get still tries Usenet and Soulseek.
    let card = resolve(artist, album).ok().flatten().unwrap_or_else(|| AlbumCard {
        id: String::new(),
        title: album.to_owned(),
        artist: artist.to_owned(),
        artist_id: None,
        cover: String::new(),
        cover_large: String::new(),
        year: query.year,
        release_date: None,
        kind: "album".to_owned(),
        track_count: None,
        explicit: false,
    });
    crate::events::ExtensionGetRequested(card.clone())
        .emit(app)
        .map_err(|error| anyhow::anyhow!("Could not reach the Flipside window: {error}"))?;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    Ok(json!({ "started": true, "onDeezer": !card.id.is_empty(), "artist": card.artist, "album": card.title }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_drops_edition_noise_only() {
        assert_eq!(simplify_title("OK Computer (Deluxe Edition)"), "OK Computer");
        assert_eq!(simplify_title("Kid A [2009 Remaster]"), "Kid A");
        assert_eq!(simplify_title("Songs (Of Love)"), "Songs (Of Love)");
    }

    #[test]
    fn tokens_are_long_and_distinct() {
        let (a, b) = (generate_token(), generate_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn only_extension_origins_are_allowed() {
        assert!(extension_origin("chrome-extension://abcdef"));
        assert!(!extension_origin("https://evil.example"));
        assert!(!extension_origin("null"));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }
}
