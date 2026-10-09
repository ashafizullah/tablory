//! Import of Navicat connection exports (File → Export Connections, `.ncx`).
//!
//! An `.ncx` file is XML with one `<Connection>` element per connection.
//! Passwords are encrypted: Navicat 12+ uses AES-128-CBC with a fixed key,
//! Navicat 11 a Blowfish-based scheme with a fixed key. Both are decrypted
//! here so imported connections keep their passwords.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::connections::{ConnectionProfile, DbKind, Secrets, SshAuth, SshConfig, SslMode};

pub struct Imported {
    pub profile: ConnectionProfile,
    pub secrets: Secrets,
}

#[derive(Serialize, Default, Debug)]
pub struct ImportReport {
    pub imported: usize,
    /// "name: reason" for each connection that was not imported.
    pub skipped: Vec<String>,
}

/// Parses an `.ncx` file. Connections of unsupported types are returned as
/// skip messages instead of failing the whole import.
pub fn parse(xml: &str) -> Result<(Vec<Imported>, Vec<String>)> {
    let doc = roxmltree::Document::parse(xml).context("not a valid Navicat .ncx file")?;
    let root = doc.root_element();
    if !root.has_tag_name("Connections") {
        bail!("not a Navicat connections export (.ncx)");
    }
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for c in root.children().filter(|n| n.has_tag_name("Connection")) {
        let a = |name: &str| c.attribute(name).unwrap_or("").trim();
        let name = a("ConnectionName").to_owned();
        let conn_type = a("ConnType");
        let Some(kind) = kind_of(conn_type) else {
            skipped.push(format!("{name}: {conn_type} is not supported"));
            continue;
        };
        let yes = |name: &str| a(name).eq_ignore_ascii_case("true");
        let secret = |name: &str| Some(decrypt(a(name))).filter(|s| !s.is_empty());

        let port = a("Port").parse().unwrap_or(0);
        let mut profile = ConnectionProfile {
            name: name.clone(),
            kind,
            color: "#6e6e73".into(),
            host: a("Host").to_owned(),
            port,
            user: a("UserName").to_owned(),
            database: a("Database").to_owned(),
            ..Default::default()
        };
        match kind {
            DbKind::Sqlite => {
                profile.file = a("DatabaseFileName").to_owned();
                profile.host.clear();
                profile.port = 0;
            }
            DbKind::Mongodb => {
                profile.auth_source =
                    first_attr(&c, &["AuthDatabase", "AuthSource", "MongoDBAuthDatabase"]);
                profile.uri = first_attr(&c, &["ConnectionString", "URI"]);
            }
            _ => {}
        }
        if yes("SSL") {
            profile.ssl_mode = SslMode::Require;
        }
        if yes("SSH") {
            profile.ssh = SshConfig {
                enabled: true,
                host: a("SSH_Host").to_owned(),
                port: a("SSH_Port").parse().unwrap_or(22),
                user: a("SSH_UserName").to_owned(),
                auth: if a("SSH_AuthenMethod").eq_ignore_ascii_case("PUBLICKEY") {
                    SshAuth::Key
                } else {
                    SshAuth::Password
                },
                key_path: a("SSH_PrivateKey").to_owned(),
            };
        }
        let secrets = Secrets {
            password: secret("Password"),
            ssh_password: secret("SSH_Password"),
            ssh_passphrase: secret("SSH_Passphrase"),
        };
        out.push(Imported { profile, secrets });
    }
    Ok((out, skipped))
}

fn first_attr(node: &roxmltree::Node, names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|n| node.attribute(*n))
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or("")
        .to_owned()
}

fn kind_of(conn_type: &str) -> Option<DbKind> {
    let t = conn_type.to_ascii_uppercase();
    Some(if t.contains("MYSQL") || t.contains("MARIADB") {
        DbKind::Mysql
    } else if t.contains("POSTGRE") {
        DbKind::Postgres
    } else if t.contains("SQLSERVER") || t.contains("MSSQL") {
        DbKind::Mssql
    } else if t.contains("SQLITE") {
        DbKind::Sqlite
    } else if t.contains("REDIS") {
        DbKind::Redis
    } else if t.contains("MONGO") {
        DbKind::Mongodb
    } else {
        return None;
    })
}

/// Decrypts a hex password from either Navicat version; empty if it fails.
fn decrypt(hex: &str) -> String {
    let Some(bytes) = from_hex(hex) else {
        return String::new();
    };
    if bytes.is_empty() {
        return String::new();
    }
    decrypt_v12(&bytes)
        .or_else(|| decrypt_v11(&bytes))
        .unwrap_or_default()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Navicat 12 and later: AES-128-CBC, PKCS#7.
fn decrypt_v12(data: &[u8]) -> Option<String> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    if !data.len().is_multiple_of(16) {
        return None;
    }
    let dec =
        cbc::Decryptor::<aes::Aes128>::new(b"libcckeylibcckey".into(), b"libcciv libcciv ".into());
    let plain = dec.decrypt_padded_vec_mut::<Pkcs7>(data).ok()?;
    String::from_utf8(plain).ok()
}

/// Navicat 11: Blowfish keyed with SHA-1("3DC5CA39") in Navicat's own
/// chaining mode, where the vector is XORed with every ciphertext block.
fn decrypt_v11(data: &[u8]) -> Option<String> {
    use blowfish::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
    use sha1::{Digest, Sha1};

    let key = Sha1::digest(b"3DC5CA39");
    let bf = <blowfish::Blowfish>::new_from_slice(&key).ok()?;
    let encrypt = |b: [u8; 8]| {
        let mut block = b.into();
        bf.encrypt_block(&mut block);
        <[u8; 8]>::from(block)
    };
    let xor = |a: [u8; 8], b: &[u8]| {
        let mut r = a;
        r.iter_mut().zip(b).for_each(|(x, y)| *x ^= y);
        r
    };

    let mut cv = encrypt([0xFF; 8]);
    let mut out = Vec::with_capacity(data.len());
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        let mut block = <[u8; 8]>::try_from(c).ok()?.into();
        bf.decrypt_block(&mut block);
        out.extend_from_slice(&xor(block.into(), &cv));
        cv = xor(cv, c);
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let pad = encrypt(cv);
        out.extend(rest.iter().zip(pad).map(|(a, b)| a ^ b));
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_both_versions() {
        assert_eq!(
            decrypt("B75D320B6211468D63EB3B67C9E85933"),
            "This is a test"
        );
        assert_eq!(decrypt("0EA71F51DD37BFB60CCBA219BE3A"), "This is a test");
        assert_eq!(decrypt(""), "");
        assert_eq!(decrypt("zz"), "");
    }

    #[test]
    fn parses_connections() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Connections Ver="1.5">
  <Connection ConnectionName="prod" ConnType="MYSQL" Host="db.example.com" Port="3307"
    Database="" UserName="root" Password="B75D320B6211468D63EB3B67C9E85933" SavePassword="true"
    SSH="true" SSH_Host="bastion" SSH_Port="2222" SSH_UserName="me" SSH_AuthenMethod="PUBLICKEY"
    SSH_PrivateKey="C:\keys\id_rsa" SSH_Password="" SSL="false"/>
  <Connection ConnectionName="local" ConnType="SQLITE" DatabaseFileName="C:\data\app.db" Password=""/>
  <Connection ConnectionName="ora" ConnType="ORACLE" Host="x" Port="1521"/>
</Connections>"#;
        let (items, skipped) = parse(xml).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(skipped, vec!["ora: ORACLE is not supported"]);

        let p = &items[0].profile;
        assert_eq!(p.kind, DbKind::Mysql);
        assert_eq!(
            (p.host.as_str(), p.port, p.user.as_str()),
            ("db.example.com", 3307, "root")
        );
        assert!(p.ssh.enabled);
        assert_eq!(p.ssh.auth, SshAuth::Key);
        assert_eq!(p.ssh.port, 2222);
        assert_eq!(items[0].secrets.password.as_deref(), Some("This is a test"));
        assert_eq!(items[0].secrets.ssh_password, None);

        assert_eq!(items[1].profile.kind, DbKind::Sqlite);
        assert_eq!(items[1].profile.file, r"C:\data\app.db");
    }

    #[test]
    fn rejects_other_xml() {
        assert!(parse("<foo/>").is_err());
        assert!(parse("not xml").is_err());
    }
}
