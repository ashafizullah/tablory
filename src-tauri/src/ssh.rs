//! SSH tunnel: a local TCP listener whose connections are forwarded through an
//! SSH server (direct-tcpip) to the database host.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use russh::client;
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::connections::{Secrets, SshAuth, SshConfig};

pub struct Tunnel {
    pub local_port: u16,
    accept_task: JoinHandle<()>,
    _handle: Arc<client::Handle<HostKeyCheck>>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Accepts keys from ~/.ssh/known_hosts, else trusts on first use and records
/// them in Tablory's own known_hosts. A changed key is always rejected.
struct HostKeyCheck {
    host: String,
    port: u16,
    app_known_hosts: PathBuf,
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = key.public_key();
        if keys::check_known_hosts(&self.host, self.port, &key)? {
            return Ok(true);
        }
        if keys::check_known_hosts_path(&self.host, self.port, &key, &self.app_known_hosts)? {
            return Ok(true);
        }
        if let Some(dir) = self.app_known_hosts.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        keys::known_hosts::learn_known_hosts_path(
            &self.host,
            self.port,
            &key,
            &self.app_known_hosts,
        )?;
        Ok(true)
    }
}

fn expand_home(p: &str) -> PathBuf {
    match (
        p.strip_prefix("~/"),
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
    ) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(p),
    }
}

fn key_file(cfg: &SshConfig) -> Result<PathBuf> {
    if !cfg.key_path.trim().is_empty() {
        return Ok(expand_home(cfg.key_path.trim()));
    }
    ["~/.ssh/id_ed25519", "~/.ssh/id_ecdsa", "~/.ssh/id_rsa"]
        .iter()
        .map(|p| expand_home(p))
        .find(|p| p.exists())
        .context("no private key found in ~/.ssh; choose a key file")
}

pub async fn open(
    cfg: &SshConfig,
    secrets: &Secrets,
    target_host: &str,
    target_port: u16,
    app_known_hosts: PathBuf,
) -> Result<Tunnel> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let check = HostKeyCheck {
        host: cfg.host.clone(),
        port: cfg.port,
        app_known_hosts,
    };
    let mut handle = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(config, (cfg.host.as_str(), cfg.port), check),
    )
    .await
    .context("SSH connection timed out")?
    .map_err(|e| match e {
        russh::Error::Keys(keys::Error::KeyChanged { .. }) => {
            anyhow::anyhow!(
                "SSH host key of {} has CHANGED; refusing to connect",
                cfg.host
            )
        }
        e => anyhow::Error::new(e).context(format!("SSH connection to {} failed", cfg.host)),
    })?;

    let auth = match cfg.auth {
        SshAuth::Password => {
            handle
                .authenticate_password(&cfg.user, secrets.ssh_password.clone().unwrap_or_default())
                .await?
        }
        SshAuth::Key => {
            let path = key_file(cfg)?;
            let key = keys::load_secret_key(&path, secrets.ssh_passphrase.as_deref())
                .with_context(|| format!("cannot read key {}", path.display()))?;
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(&cfg.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await?
        }
    };
    if !auth.success() {
        bail!("SSH authentication failed for {}@{}", cfg.user, cfg.host);
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let local_port = listener.local_addr()?.port();
    let handle = Arc::new(handle);
    let h = handle.clone();
    let target_host = target_host.to_owned();
    let accept_task = tokio::spawn(async move {
        while let Ok((mut socket, peer)) = listener.accept().await {
            let h = h.clone();
            let target_host = target_host.clone();
            tokio::spawn(async move {
                let channel = h
                    .channel_open_direct_tcpip(
                        target_host,
                        target_port.into(),
                        "127.0.0.1",
                        peer.port().into(),
                    )
                    .await;
                if let Ok(channel) = channel {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                }
            });
        }
    });

    Ok(Tunnel {
        local_port,
        accept_task,
        _handle: handle,
    })
}
