use keyring::{Entry, Error as KeyringError};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use zeroize::Zeroizing;

const CREDENTIAL_SERVICE: &str = "com.soundtrackgeek.flipside.soulseek";

type SessionCredential = Option<(String, Zeroizing<String>)>;

#[derive(Clone, Default)]
pub struct CredentialVault {
    session: Arc<Mutex<SessionCredential>>,
}

impl CredentialVault {
    pub fn store(
        &self,
        username: &str,
        password: String,
        remember_password: bool,
    ) -> Result<(), CredentialError> {
        if password.is_empty() {
            return Err(CredentialError::Missing);
        }

        if remember_password {
            entry(username)?.set_password(&password)?;
        } else {
            delete_native(username)?;
        }

        *self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((username.to_owned(), Zeroizing::new(password)));
        Ok(())
    }

    pub fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, CredentialError> {
        let session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some((session_username, password)) = session.as_ref() {
            if session_username == username {
                return Ok(Some(Zeroizing::new(password.to_string())));
            }
        }
        drop(session);

        match entry(username)?.get_password() {
            Ok(password) => Ok(Some(Zeroizing::new(password))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn has(&self, username: &str) -> Result<bool, CredentialError> {
        Ok(self.get(username)?.is_some())
    }

    pub fn forget(&self, username: &str) -> Result<(), CredentialError> {
        {
            let mut session = self
                .session
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if session
                .as_ref()
                .is_some_and(|(session_username, _)| session_username == username)
            {
                *session = None;
            }
        }

        delete_native(username)?;
        Ok(())
    }
}

fn entry(username: &str) -> Result<Entry, CredentialError> {
    Entry::new(CREDENTIAL_SERVICE, username).map_err(Into::into)
}

fn delete_native(username: &str) -> Result<(), CredentialError> {
    match entry(username)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Enter your Soulseek password.")]
    Missing,
    #[error("Windows Credential Manager could not be accessed: {0}")]
    Keyring(#[from] KeyringError),
}
