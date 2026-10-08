//! Saved connection profiles (connections.json) and their secrets, which live
//! in the OS keychain rather than on disk.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    #[default]
    Postgres,
    Mysql,
    Sqlite,
    Mssql,
    Redis,
    Mongodb,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    #[default]
    Password,
    Key,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SshConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// Private key file; empty tries ~/.ssh/id_ed25519 then ~/.ssh/id_rsa.
    pub key_path: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: SshAuth::Password,
            key_path: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    /// Tag color shown in the connection list and title bar.
    pub color: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    /// SQLite database file.
    pub file: String,
    pub ssl_mode: SslMode,
    pub ssh: SshConfig,
    /// MongoDB: full connection string; when set it replaces host, user and SSL.
    pub uri: String,
    /// MongoDB: database the user is defined in (default "admin").
    pub auth_source: String,
}

/// Passwords sent by the connection form. `None` keeps the stored value.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Secrets {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
}

impl Secrets {
    /// Fields set here win over `stored`.
    pub fn merged_over(self, stored: Secrets) -> Secrets {
        Secrets {
            password: self.password.or(stored.password),
            ssh_password: self.ssh_password.or(stored.ssh_password),
            ssh_passphrase: self.ssh_passphrase.or(stored.ssh_passphrase),
        }
    }
}

pub struct Store {
    file: PathBuf,
}

impl Store {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            file: config_dir.join("connections.json"),
        }
    }

    pub fn list(&self) -> Result<Vec<ConnectionProfile>> {
        match std::fs::read_to_string(&self.file) {
            Ok(s) => serde_json::from_str(&s).context("connections.json is corrupt"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(&self, id: &str) -> Result<ConnectionProfile> {
        self.list()?
            .into_iter()
            .find(|p| p.id == id)
            .context("connection not found")
    }

    fn write(&self, all: &[ConnectionProfile]) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(all)?)?;
        std::fs::rename(tmp, &self.file)?;
        Ok(())
    }

    pub fn save(&self, mut profile: ConnectionProfile) -> Result<ConnectionProfile> {
        let mut all = self.list()?;
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        }
        match all.iter_mut().find(|p| p.id == profile.id) {
            Some(p) => *p = profile.clone(),
            None => all.push(profile.clone()),
        }
        self.write(&all)?;
        Ok(profile)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut all = self.list()?;
        all.retain(|p| p.id != id);
        self.write(&all)
    }
}

const KEYRING_SERVICE: &str = "app.tablory";

fn entry(id: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, id)?)
}

/// All secrets of a connection are one keychain item, so macOS asks once.
pub fn load_secrets(id: &str) -> Secrets {
    entry(id)
        .and_then(|e| Ok(e.get_password()?))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Stores `secrets` over the existing ones; empty strings clear a field.
pub fn save_secrets(id: &str, secrets: Secrets) -> Result<()> {
    let merged = secrets.merged_over(load_secrets(id));
    let clean = |v: Option<String>| v.filter(|s| !s.is_empty());
    let merged = Secrets {
        password: clean(merged.password),
        ssh_password: clean(merged.ssh_password),
        ssh_passphrase: clean(merged.ssh_passphrase),
    };
    let e = entry(id)?;
    if merged.password.is_none() && merged.ssh_password.is_none() && merged.ssh_passphrase.is_none()
    {
        let _ = e.delete_credential();
        return Ok(());
    }
    e.set_password(&serde_json::to_string(&merged)?)
        .context("could not save the password to the system keychain")
}

pub fn delete_secrets(id: &str) {
    if let Ok(e) = entry(id) {
        let _ = e.delete_credential();
    }
}
