//! Open connections. A session owns its driver and, when configured, the SSH
//! tunnel the driver connects through.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::connections::{ConnectionProfile, DbKind, Safety, Secrets};
use crate::db::{self, Driver};
use crate::docdb::MongoDriver;
use crate::kv::RedisDriver;
use crate::ssh::{self, Tunnel};

/// The connected engine: SQL engines share [`Driver`]; Redis and MongoDB
/// have their own shapes.
#[derive(Clone)]
pub enum Backend {
    Sql(Arc<dyn Driver>),
    Redis(Arc<RedisDriver>),
    Mongo(Arc<MongoDriver>),
}

impl Backend {
    async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        endpoint: (String, u16),
        database: &str,
    ) -> Result<Self> {
        Ok(match profile.kind {
            DbKind::Redis => Backend::Redis(Arc::new(
                RedisDriver::connect(profile, secrets, endpoint, database).await?,
            )),
            DbKind::Mongodb => {
                let mut p = profile.clone();
                p.database = database.to_owned();
                Backend::Mongo(Arc::new(MongoDriver::connect(&p, secrets, endpoint).await?))
            }
            _ => Backend::Sql(db::connect(profile, secrets, endpoint, database).await?),
        })
    }

    pub async fn current_database(&self) -> Result<String> {
        match self {
            Backend::Sql(d) => d.current_database().await,
            Backend::Redis(r) => Ok(r.db().to_string()),
            Backend::Mongo(m) => Ok(m.current_database()),
        }
    }

    async fn close(&self) {
        match self {
            Backend::Sql(d) => d.close().await,
            Backend::Redis(_) => {}
            Backend::Mongo(m) => m.close().await,
        }
    }

    pub fn sql(&self) -> Result<Arc<dyn Driver>> {
        match self {
            Backend::Sql(d) => Ok(d.clone()),
            _ => anyhow::bail!("not a SQL connection"),
        }
    }

    pub fn redis(&self) -> Result<Arc<RedisDriver>> {
        match self {
            Backend::Redis(r) => Ok(r.clone()),
            _ => anyhow::bail!("not a Redis connection"),
        }
    }

    pub fn mongo(&self) -> Result<Arc<MongoDriver>> {
        match self {
            Backend::Mongo(m) => Ok(m.clone()),
            _ => anyhow::bail!("not a MongoDB connection"),
        }
    }
}

pub struct Session {
    pub profile: ConnectionProfile,
    secrets: Secrets,
    endpoint: (String, u16),
    backend: RwLock<Backend>,
    _tunnel: Option<Tunnel>,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub connection_id: String,
    pub name: String,
    pub kind: DbKind,
    pub color: String,
    pub database: String,
    pub safety: Safety,
}

impl Session {
    pub async fn backend(&self) -> Backend {
        self.backend.read().await.clone()
    }

    /// The SQL driver; errors for Redis and MongoDB sessions.
    pub async fn driver(&self) -> Result<Arc<dyn Driver>> {
        self.backend().await.sql()
    }
}

fn default_port(kind: DbKind) -> u16 {
    match kind {
        DbKind::Postgres => 5432,
        DbKind::Mysql => 3306,
        DbKind::Mssql => 1433,
        DbKind::Redis => 6379,
        DbKind::Mongodb => 27017,
        DbKind::Sqlite => 0,
    }
}

/// Opens the tunnel (if any) and the driver.
async fn open(
    profile: &ConnectionProfile,
    secrets: &Secrets,
    known_hosts: PathBuf,
) -> Result<(Backend, Option<Tunnel>, (String, u16))> {
    let host = if profile.host.trim().is_empty() {
        "127.0.0.1".to_owned()
    } else {
        profile.host.trim().to_owned()
    };
    let port = if profile.port == 0 {
        default_port(profile.kind)
    } else {
        profile.port
    };
    // A MongoDB connection string names its own hosts, so no tunnel.
    let uses_uri = profile.kind == DbKind::Mongodb && !profile.uri.trim().is_empty();
    let (tunnel, endpoint) = if profile.ssh.enabled && profile.kind != DbKind::Sqlite && !uses_uri {
        // SQL Server "server\INSTANCE": the tunnel goes to the server; the
        // instance's port must be set on the profile.
        let target = host.split('\\').next().unwrap_or(&host);
        let t = ssh::open(&profile.ssh, secrets, target, port, known_hosts).await?;
        let endpoint = ("127.0.0.1".to_owned(), t.local_port);
        (Some(t), endpoint)
    } else {
        (None, (host, port))
    };
    let driver = Backend::connect(profile, secrets, endpoint.clone(), &profile.database).await?;
    Ok((driver, tunnel, endpoint))
}

pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    known_hosts: PathBuf,
}

impl Sessions {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            map: Mutex::default(),
            known_hosts: config_dir.join("known_hosts"),
        }
    }

    /// Connects and immediately disconnects.
    pub async fn test(&self, profile: &ConnectionProfile, secrets: &Secrets) -> Result<String> {
        let (driver, _tunnel, _) = open(profile, secrets, self.known_hosts.clone()).await?;
        let db = driver.current_database().await;
        driver.close().await;
        db
    }

    pub async fn connect(
        &self,
        profile: ConnectionProfile,
        secrets: Secrets,
    ) -> Result<SessionInfo> {
        let (driver, tunnel, endpoint) = open(&profile, &secrets, self.known_hosts.clone()).await?;
        let database = driver.current_database().await?;
        let id = uuid::Uuid::new_v4().to_string();
        let info = SessionInfo {
            id: id.clone(),
            connection_id: profile.id.clone(),
            name: profile.name.clone(),
            kind: profile.kind,
            color: profile.color.clone(),
            database,
            safety: profile.safety,
        };
        let session = Session {
            profile,
            secrets,
            endpoint,
            backend: RwLock::new(driver),
            _tunnel: tunnel,
        };
        self.map.lock().await.insert(id, Arc::new(session));
        Ok(info)
    }

    pub async fn get(&self, id: &str) -> Result<Arc<Session>> {
        self.map
            .lock()
            .await
            .get(id)
            .cloned()
            .context("not connected (the session was closed)")
    }

    /// Reconnects the session's driver to another database on the same server,
    /// reusing the tunnel.
    pub async fn switch_database(&self, id: &str, database: &str) -> Result<String> {
        let s = self.get(id).await?;
        let new = Backend::connect(&s.profile, &s.secrets, s.endpoint.clone(), database).await?;
        let name = new.current_database().await?;
        let old = std::mem::replace(&mut *s.backend.write().await, new);
        old.close().await;
        Ok(name)
    }

    pub async fn disconnect(&self, id: &str) {
        let s = self.map.lock().await.remove(id);
        if let Some(s) = s {
            s.backend().await.close().await;
        }
    }

    pub async fn close_all(&self) {
        let all: Vec<_> = self.map.lock().await.drain().collect();
        for (_, s) in all {
            s.backend().await.close().await;
        }
    }
}
