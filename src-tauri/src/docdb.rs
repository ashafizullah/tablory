//! MongoDB: databases, collections, paged finds with JSON filters, and
//! document insert / replace / delete by `_id`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::results::CollectionType;
use mongodb::Client;
use serde::Serialize;
use serde_json::Value;

use crate::connections::{ConnectionProfile, Secrets, SslMode};
use crate::db::TableInfo;

pub struct MongoDriver {
    client: Client,
    database: String,
}

#[derive(Serialize)]
pub struct FindResult {
    /// Relaxed Extended JSON, for the grid.
    pub docs: Vec<Value>,
    /// The same documents pretty-printed, for editing; keeps 1.0 vs 1 apart.
    pub texts: Vec<String>,
}

/// Lenient JSON (unquoted keys, single quotes, trailing commas) to BSON;
/// Extended JSON such as {"$oid": "…"} or {"$date": "…"} is understood.
pub fn parse_bson(text: &str) -> Result<Bson> {
    let v: Value = json5::from_str(text.trim())
        .context("expected JSON, e.g. {\"status\": \"active\"} or {\"_id\": {\"$oid\": \"…\"}}")?;
    Bson::try_from(v).context("invalid Extended JSON")
}

pub fn parse_doc(text: &str) -> Result<Document> {
    if text.trim().is_empty() {
        return Ok(Document::new());
    }
    match parse_bson(text)? {
        Bson::Document(d) => Ok(d),
        _ => bail!("expected a JSON object"),
    }
}

fn to_json(d: Document) -> Value {
    Bson::Document(d).into_relaxed_extjson()
}

impl MongoDriver {
    pub async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        (host, port): (String, u16),
    ) -> Result<Self> {
        let mut options = if !profile.uri.trim().is_empty() {
            ClientOptions::parse(profile.uri.trim()).await?
        } else {
            let mut o = ClientOptions::default();
            o.hosts = vec![ServerAddress::Tcp {
                host,
                port: Some(if port == 0 { 27017 } else { port }),
            }];
            // Talk to this server only; replica-set discovery would try hosts
            // that are unreachable through an SSH tunnel.
            o.direct_connection = Some(true);
            if !profile.user.is_empty() {
                let source = if profile.auth_source.trim().is_empty() {
                    "admin".to_owned()
                } else {
                    profile.auth_source.trim().to_owned()
                };
                o.credential = Some(
                    Credential::builder()
                        .username(profile.user.clone())
                        .password(secrets.password.clone().unwrap_or_default())
                        .source(source)
                        .build(),
                );
            }
            if profile.ssl_mode == SslMode::Require {
                o.tls = Some(Tls::Enabled(
                    TlsOptions::builder()
                        .allow_invalid_certificates(true)
                        .build(),
                ));
            }
            o
        };
        options.app_name = Some("Tablory".into());
        options.server_selection_timeout = Some(Duration::from_secs(10));
        options.connect_timeout = Some(Duration::from_secs(10));
        let database = options
            .default_database
            .clone()
            .filter(|_| profile.database.is_empty())
            .unwrap_or_else(|| profile.database.clone());
        let client = Client::with_options(options)?;
        client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await?;
        Ok(Self { client, database })
    }

    pub fn current_database(&self) -> String {
        self.database.clone()
    }

    pub async fn databases(&self) -> Result<Vec<String>> {
        let mut names = self.client.list_database_names().await?;
        names.sort();
        Ok(names)
    }

    pub async fn collections(&self, db: &str) -> Result<Vec<TableInfo>> {
        let specs: Vec<_> = self
            .client
            .database(db)
            .list_collections()
            .await?
            .try_collect()
            .await?;
        let mut out: Vec<TableInfo> = specs
            .into_iter()
            .filter(|s| !s.name.starts_with("system."))
            .map(|s| TableInfo {
                kind: if matches!(s.collection_type, CollectionType::View) {
                    "view".into()
                } else {
                    "table".into()
                },
                name: s.name,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn coll(&self, db: &str, coll: &str) -> mongodb::Collection<Document> {
        self.client.database(db).collection(coll)
    }

    pub async fn find(
        &self,
        db: &str,
        coll: &str,
        filter: &str,
        sort: &str,
        skip: u64,
        limit: i64,
    ) -> Result<FindResult> {
        let docs: Vec<Document> = self
            .coll(db, coll)
            .find(parse_doc(filter)?)
            .sort(parse_doc(sort)?)
            .skip(skip)
            .limit(limit)
            .await?
            .try_collect()
            .await?;
        let docs: Vec<Value> = docs.into_iter().map(to_json).collect();
        let texts = docs
            .iter()
            .map(|d| serde_json::to_string_pretty(d).unwrap_or_default())
            .collect();
        Ok(FindResult { docs, texts })
    }

    pub async fn count(&self, db: &str, coll: &str, filter: &str) -> Result<u64> {
        let filter = parse_doc(filter)?;
        let c = self.coll(db, coll);
        Ok(if filter.is_empty() {
            c.estimated_document_count().await?
        } else {
            c.count_documents(filter).await?
        })
    }

    pub async fn insert(&self, db: &str, coll: &str, doc: &str) -> Result<Value> {
        let res = self.coll(db, coll).insert_one(parse_doc(doc)?).await?;
        Ok(res.inserted_id.into_relaxed_extjson())
    }

    /// Replaces the document with this `_id`; the new text may not change `_id`.
    pub async fn replace(&self, db: &str, coll: &str, id: &str, doc: &str) -> Result<()> {
        let id = parse_bson(id)?;
        let mut new = parse_doc(doc)?;
        match new.get("_id") {
            Some(v) if *v != id => bail!("_id cannot be changed; insert a new document instead"),
            _ => {
                new.remove("_id");
            }
        }
        let res = self
            .coll(db, coll)
            .replace_one(doc! { "_id": id }, new)
            .await?;
        if res.matched_count != 1 {
            bail!("the document no longer exists; reload and try again");
        }
        Ok(())
    }

    pub async fn delete(&self, db: &str, coll: &str, ids: &[String]) -> Result<u64> {
        let ids = ids
            .iter()
            .map(|i| parse_bson(i))
            .collect::<Result<Vec<_>>>()?;
        let res = self
            .coll(db, coll)
            .delete_many(doc! { "_id": { "$in": ids } })
            .await?;
        Ok(res.deleted_count)
    }

    pub async fn command(&self, db: &str, command: &str) -> Result<Value> {
        let res = self
            .client
            .database(db)
            .run_command(parse_doc(command)?)
            .await?;
        Ok(to_json(res))
    }

    pub async fn close(&self) {
        self.client.clone().shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_filters() {
        let d = parse_doc("{status: 'active', n: {$gt: 2},}").unwrap();
        assert_eq!(d, doc! { "status": "active", "n": { "$gt": 2 } });
        let d = parse_doc(r#"{"_id": {"$oid": "65f000000000000000000001"}}"#).unwrap();
        assert!(matches!(d.get("_id"), Some(Bson::ObjectId(_))));
        assert!(parse_doc("").unwrap().is_empty());
        assert!(parse_doc("[1]").is_err());
    }
}
