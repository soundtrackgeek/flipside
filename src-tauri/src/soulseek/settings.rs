use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const DEFAULT_SERVER_HOST: &str = "server.slsknet.org";
pub const DEFAULT_SERVER_PORT: u16 = 2242;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub username: String,
    pub server_host: String,
    pub server_port: u16,
    pub download_directory: String,
    pub remember_password: bool,
    pub auto_connect: bool,
}

impl ConnectionProfile {
    pub fn suggested(download_directory: &Path) -> Self {
        Self {
            username: "jtbleach3".to_owned(),
            server_host: DEFAULT_SERVER_HOST.to_owned(),
            server_port: DEFAULT_SERVER_PORT,
            download_directory: download_directory.to_string_lossy().into_owned(),
            remember_password: true,
            auto_connect: true,
        }
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.username.is_empty() {
            return Err(SettingsError::Validation(
                "Enter a Soulseek username.".to_owned(),
            ));
        }

        if self.username.len() > 30 {
            return Err(SettingsError::Validation(
                "Soulseek usernames can be at most 30 characters.".to_owned(),
            ));
        }

        if self.username.trim() != self.username {
            return Err(SettingsError::Validation(
                "Remove leading or trailing spaces from the username.".to_owned(),
            ));
        }

        if !self
            .username
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        {
            return Err(SettingsError::Validation(
                "Use printable ASCII characters in the username.".to_owned(),
            ));
        }

        if self.server_host.is_empty()
            || self.server_host.len() > 255
            || self.server_host.chars().any(char::is_whitespace)
        {
            return Err(SettingsError::Validation(
                "Enter a valid Soulseek server hostname.".to_owned(),
            ));
        }

        if self.server_port == 0 {
            return Err(SettingsError::Validation(
                "Enter a valid Soulseek server port.".to_owned(),
            ));
        }

        if self.download_directory.trim().is_empty() {
            return Err(SettingsError::Validation(
                "Choose a download folder.".to_owned(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<ConnectionProfile>, SettingsError> {
        if !self.path.exists() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(&self.path)?;
        let profile = serde_json::from_str::<ConnectionProfile>(&contents)?;
        profile.validate()?;
        Ok(Some(profile))
    }

    pub fn save(&self, profile: &ConnectionProfile) -> Result<(), SettingsError> {
        profile.validate()?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::create_dir_all(&profile.download_directory)?;
        let contents = serde_json::to_string_pretty(profile)?;
        std::fs::write(&self.path, contents)?;
        Ok(())
    }

    pub fn delete(&self) -> Result<(), SettingsError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("{0}")]
    Validation(String),
    #[error("Could not read or save connection settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection settings are not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_usernames() {
        let mut profile = ConnectionProfile::suggested(Path::new("C:\\Downloads\\Flipside"));
        profile.username = " leading".to_owned();

        assert!(matches!(
            profile.validate(),
            Err(SettingsError::Validation(message)) if message.contains("leading or trailing")
        ));

        profile.username = "snow☃".to_owned();
        assert!(matches!(
            profile.validate(),
            Err(SettingsError::Validation(message)) if message.contains("printable ASCII")
        ));
    }

    #[test]
    fn round_trips_profile_without_a_password() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = SettingsStore::new(directory.path().join("connection.json"));
        let mut profile = ConnectionProfile::suggested(&directory.path().join("downloads"));
        profile.username = "SignalLevel".to_owned();

        store.save(&profile).expect("save settings");
        let saved = store.load().expect("load settings").expect("saved profile");

        assert_eq!(saved, profile);
        let raw = std::fs::read_to_string(store.path()).expect("settings contents");
        assert!(!raw.contains("\"password\""));
    }
}
