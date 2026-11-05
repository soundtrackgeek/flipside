//! API keys for the metadata providers (Last.fm, Discogs, Fanart.tv).
//!
//! Values live in Windows Credential Manager. On startup, anything found in a `.env`
//! file (next to the executable, the working directory, or a parent of it) is copied
//! into the vault for providers that are not configured yet — `.env` is a dev/bootstrap
//! convenience, never the source of truth once a key is stored.

use anyhow::{Context, Result};
use keyring::Entry;
use serde::Serialize;
use zeroize::Zeroizing;

const SERVICE: &str = "com.soundtrackgeek.flipside.keys";

/// (id, label, env name)
pub const KEYS: &[(&str, &str, &str)] = &[
    ("lastfm_key", "Last.fm API key", "LAST_FM"),
    ("lastfm_secret", "Last.fm shared secret", "LAST_FM_SECRET"),
    ("discogs_key", "Discogs consumer key", "DISCOGS_CONSUMER_KEY"),
    ("discogs_secret", "Discogs consumer secret", "DISCOGS_CONSUMER_SECRET"),
    ("fanart_key", "Fanart.tv API key", "FANART_TV"),
];

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub id: String,
    pub label: String,
    pub configured: bool,
}

fn entry(id: &str) -> Result<Entry> {
    Entry::new(SERVICE, id).context("Could not open Windows Credential Manager")
}

pub fn get(id: &str) -> Option<Zeroizing<String>> {
    match entry(id).ok()?.get_password() {
        Ok(value) if !value.trim().is_empty() => Some(Zeroizing::new(value)),
        _ => None,
    }
}

pub fn status() -> Vec<KeyStatus> {
    KEYS.iter()
        .map(|(id, label, _)| KeyStatus {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            configured: get(id).is_some(),
        })
        .collect()
}

fn known(id: &str) -> Result<()> {
    KEYS.iter()
        .any(|(known, _, _)| *known == id)
        .then_some(())
        .context("Unknown provider key")
}

pub fn save(id: &str, value: &str) -> Result<Vec<KeyStatus>> {
    known(id)?;
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("Enter a value first.");
    }
    entry(id)?
        .set_password(value)
        .context("Could not save the key in Windows Credential Manager")?;
    Ok(status())
}

pub fn delete(id: &str) -> Result<Vec<KeyStatus>> {
    known(id)?;
    match entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(error).context("Could not remove the key"),
    }
    Ok(status())
}

fn env_candidates() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        roots.extend(exe.ancestors().skip(1).take(5).map(|p| p.to_path_buf()));
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.extend(cwd.ancestors().take(4).map(|p| p.to_path_buf()));
    }
    roots.into_iter().map(|root| root.join(".env")).filter(|p| p.is_file()).collect()
}

/// Copies provider keys from a `.env` file into the vault when they are missing.
pub fn seed_from_env() {
    let Some(path) = env_candidates().into_iter().next() else { return };
    let Ok(iter) = dotenvy::from_path_iter(&path) else { return };
    let vars: std::collections::HashMap<String, String> = iter.flatten().collect();
    for (id, _, env_name) in KEYS {
        if let Some(value) = vars.get(*env_name).filter(|v| !v.trim().is_empty()) {
            if get(id).is_none() {
                let _ = save(id, value);
            }
        }
    }
    if let Some(arl) = vars.get("DEEMIX_ARL") {
        let _ = crate::deemix::seed_arl_if_missing(arl);
    }
    if let Some(key) = vars.get("PROWLARR_API") {
        let _ = crate::usenet::seed_prowlarr_key_if_missing(key);
    }
}
