//! Saved connection profiles (connections.json) and their secrets, which live
//! in the OS keychain rather than on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
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

/// Guard rails for SQL connections.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    #[default]
    Normal,
    /// Statements and edits that change data ask for confirmation.
    Production,
    /// Statements and edits that change data are blocked.
    Readonly,
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
    /// Folder in the connection list; empty means ungrouped.
    pub group: String,
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
    /// SQL Server: log in as the current Windows user instead of user/password.
    pub windows_auth: bool,
    pub safety: Safety,
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
    /// Group names in display order, so empty groups survive.
    groups_file: PathBuf,
    /// Group name -> tag color.
    group_colors_file: PathBuf,
}

impl Store {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            file: config_dir.join("connections.json"),
            groups_file: config_dir.join("groups.json"),
            group_colors_file: config_dir.join("group_colors.json"),
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
        write_json(&self.file, all)
    }

    /// Saved groups, then any group a connection names that is not saved.
    pub fn groups(&self) -> Result<Vec<String>> {
        let mut groups: Vec<String> = match std::fs::read_to_string(&self.groups_file) {
            Ok(s) => serde_json::from_str(&s).context("groups.json is corrupt")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let mut extra: Vec<String> = self
            .list()?
            .into_iter()
            .map(|p| p.group)
            .filter(|g| !g.is_empty() && !groups.contains(g))
            .collect();
        extra.sort();
        extra.dedup();
        groups.extend(extra);
        Ok(groups)
    }

    pub fn group_colors(&self) -> Result<HashMap<String, String>> {
        match std::fs::read_to_string(&self.group_colors_file) {
            Ok(s) => serde_json::from_str(&s).context("group_colors.json is corrupt"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// An empty color clears it.
    pub fn set_group_color(&self, name: &str, color: &str) -> Result<()> {
        let mut colors = self.group_colors()?;
        if color.is_empty() {
            colors.remove(name);
        } else {
            colors.insert(name.to_owned(), color.to_owned());
        }
        write_json(&self.group_colors_file, &colors)
    }

    pub fn create_group(&self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("the group name is empty");
        }
        let mut groups = self.groups()?;
        if groups.iter().any(|g| g == name) {
            bail!("a group named \"{name}\" already exists");
        }
        groups.push(name.to_owned());
        write_json(&self.groups_file, &groups)?;
        Ok(name.to_owned())
    }

    /// Renaming onto an existing group merges the two.
    pub fn rename_group(&self, from: &str, to: &str) -> Result<()> {
        let to = to.trim();
        if to.is_empty() {
            bail!("the group name is empty");
        }
        let mut groups = self.groups()?;
        if groups.iter().any(|g| g == to) {
            groups.retain(|g| g != from);
        } else if let Some(g) = groups.iter_mut().find(|g| *g == from) {
            *g = to.to_owned();
        }
        write_json(&self.groups_file, &groups)?;
        // The color follows the rename unless merging into a colored group.
        let mut colors = self.group_colors()?;
        if let Some(c) = colors.remove(from) {
            colors.entry(to.to_owned()).or_insert(c);
            write_json(&self.group_colors_file, &colors)?;
        }
        self.regroup(from, to)
    }

    /// Removes the group; its connections become ungrouped, not deleted.
    pub fn delete_group(&self, name: &str) -> Result<()> {
        let mut groups = self.groups()?;
        groups.retain(|g| g != name);
        write_json(&self.groups_file, &groups)?;
        self.set_group_color(name, "")?;
        self.regroup(name, "")
    }

    pub fn move_connection(&self, id: &str, group: &str) -> Result<()> {
        let mut all = self.list()?;
        let p = all
            .iter_mut()
            .find(|p| p.id == id)
            .context("connection not found")?;
        p.group = group.trim().to_owned();
        self.write(&all)
    }

    fn regroup(&self, from: &str, to: &str) -> Result<()> {
        let mut all = self.list()?;
        for p in all.iter_mut().filter(|p| p.group == from) {
            p.group = to.to_owned();
        }
        self.write(&all)
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

/// Writes through a temp file so a crash never leaves half a file.
fn write_json<T: Serialize + ?Sized>(file: &Path, value: &T) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(tmp, file)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_lifecycle() {
        let dir = std::env::temp_dir().join(format!("tablory-test-{}", uuid::Uuid::new_v4()));
        let s = Store::new(&dir);
        let c = s
            .save(ConnectionProfile {
                name: "a".into(),
                group: "Legacy".into(),
                ..Default::default()
            })
            .unwrap();
        // A group only named by a connection still shows up.
        assert_eq!(s.groups().unwrap(), ["Legacy"]);

        s.create_group("Prod").unwrap();
        assert!(s.create_group(" Prod ").is_err());
        assert!(s.create_group("  ").is_err());
        assert_eq!(s.groups().unwrap(), ["Legacy", "Prod"]);

        s.move_connection(&c.id, "Prod").unwrap();
        s.rename_group("Prod", "Production").unwrap();
        assert_eq!(s.get(&c.id).unwrap().group, "Production");
        assert_eq!(s.groups().unwrap(), ["Legacy", "Production"]);

        s.create_group("Dev").unwrap();
        s.rename_group("Dev", "Production").unwrap();
        assert_eq!(s.groups().unwrap(), ["Legacy", "Production"]);

        s.delete_group("Production").unwrap();
        assert_eq!(s.get(&c.id).unwrap().group, "");
        assert_eq!(s.groups().unwrap(), ["Legacy"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
