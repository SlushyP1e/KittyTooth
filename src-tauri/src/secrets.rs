//! Tokens and API keys in the OS credential store (Windows Credential Manager via `keyring`).
//! Values never touch config.json, the logs or "Copy diagnostics".

use kittytooth_core::source::{SecretKind, SecretStore};
use kittytooth_core::{KtError, KtResult};

const SERVICE: &str = "KittyTooth";

pub struct KeyringSecrets;

fn entry(kind: SecretKind) -> KtResult<keyring::Entry> {
    keyring::Entry::new(SERVICE, kind.account()).map_err(|e| KtError::Io(format!("credential store: {e}")))
}

impl SecretStore for KeyringSecrets {
    fn get(&self, kind: SecretKind) -> Option<String> {
        match entry(kind).ok()?.get_password() {
            Ok(v) => Some(v),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => {
                tracing::warn!(account = kind.account(), error = %e, "could not read credential");
                None
            }
        }
    }

    fn set(&self, kind: SecretKind, value: &str) -> KtResult<()> {
        entry(kind)?
            .set_password(value)
            .map_err(|e| KtError::Io(format!("credential store: {e}")))
    }

    fn delete(&self, kind: SecretKind) -> KtResult<()> {
        match entry(kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KtError::Io(format!("credential store: {e}"))),
        }
    }
}

/// Replaces any stored secret that appears in `text` with its masked form.
pub fn scrub(text: &str, store: &dyn SecretStore) -> String {
    let mut out = text.to_string();
    for kind in SecretKind::ALL {
        if let Some(v) = store.get(kind).filter(|v| v.len() >= 4) {
            out = out.replace(&v, &kittytooth_core::source::mask(&v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittytooth_core::source::MemorySecrets;

    #[test]
    fn scrubs_secrets_from_text() {
        let m = MemorySecrets::default();
        m.set(SecretKind::PulsoidToken, "8c4da3ce-7ed7-4a19").unwrap();
        let t = scrub("url ?access_token=8c4da3ce-7ed7-4a19&x", &m);
        assert!(!t.contains("8c4da3ce"));
        assert!(t.contains("••••4a19"));
    }
}
