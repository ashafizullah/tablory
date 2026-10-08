//! Redis: key browsing with SCAN, typed value reads, and a raw command console.
//! Edits from the UI are plain Redis commands sent through [`RedisDriver::command`].

use anyhow::{bail, Context, Result};
use redis::aio::ConnectionManager;
use redis::Value as RValue;
use serde::Serialize;
use serde_json::{json, Value};

use crate::connections::{ConnectionProfile, Secrets, SslMode};

/// Values longer than this are cut when listing collections (hash, list…).
const MAX_ITEMS: usize = 1000;

pub struct RedisDriver {
    conn: ConnectionManager,
    db: i64,
}

#[derive(Serialize)]
pub struct KeyEntry {
    pub key: String,
    pub kind: String,
}

#[derive(Serialize)]
pub struct ScanPage {
    /// "0" when the scan is complete.
    pub cursor: String,
    pub keys: Vec<KeyEntry>,
}

#[derive(Serialize)]
pub struct KeyValue {
    pub key: String,
    pub kind: String,
    /// Seconds; -1 = no expiry, -2 = key does not exist.
    pub ttl: i64,
    /// Number of items (or bytes for strings).
    pub length: u64,
    /// string: text; hash: [[field, value]]; list/set: [value]; zset: [[member, score]];
    /// stream: [[id, {field: value}]].
    pub value: Value,
    /// Only the first MAX_ITEMS items were read.
    pub truncated: bool,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Text, or `{"$bin": len, "hex": …}` for bytes that are not UTF-8.
fn bytes_json(b: &[u8]) -> Value {
    match std::str::from_utf8(b) {
        Ok(s) => Value::String(s.to_owned()),
        Err(_) => crate::db::binary_cell(b),
    }
}

/// Any reply as JSON, for the console.
pub fn reply_json(v: &RValue) -> Value {
    match v {
        RValue::Nil => Value::Null,
        RValue::Int(n) => json!(n),
        RValue::BulkString(b) => bytes_json(b),
        RValue::Array(items) | RValue::Set(items) => {
            Value::Array(items.iter().map(reply_json).collect())
        }
        RValue::SimpleString(s) => Value::String(s.clone()),
        RValue::Okay => Value::String("OK".into()),
        RValue::Map(pairs) => Value::Array(
            pairs
                .iter()
                .map(|(k, v)| Value::Array(vec![reply_json(k), reply_json(v)]))
                .collect(),
        ),
        RValue::Double(f) => json!(f),
        RValue::Boolean(b) => json!(b),
        RValue::VerbatimString { text, .. } => Value::String(text.clone()),
        other => Value::String(format!("{other:?}")),
    }
}

fn text(v: &RValue) -> String {
    match reply_json(v) {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

fn pairs(v: &RValue) -> Vec<(RValue, RValue)> {
    match v {
        RValue::Map(p) => p.clone(),
        RValue::Array(items) => items
            .chunks(2)
            .filter_map(|c| match c {
                [a, b] => Some((a.clone(), b.clone())),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Splits a console line into arguments, honoring quotes and backslash escapes.
pub fn split_args(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some('n') => cur.push('\n'),
                Some('t') => cur.push('\t'),
                Some(o) => cur.push(o),
                None => bail!("unfinished escape"),
            },
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        bail!("unterminated quote");
    }
    if started {
        args.push(cur);
    }
    Ok(args)
}

impl RedisDriver {
    pub async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        (host, port): (String, u16),
        database: &str,
    ) -> Result<Self> {
        let db: i64 = if database.trim().is_empty() {
            0
        } else {
            database
                .trim()
                .parse()
                .context("the Redis database must be a number")?
        };
        let auth = match (profile.user.as_str(), secrets.password.as_deref()) {
            (_, None | Some("")) if profile.user.is_empty() => String::new(),
            (user, pass) => format!("{}:{}@", encode(user), encode(pass.unwrap_or(""))),
        };
        let port = if port == 0 { 6379 } else { port };
        // Like the SQL drivers, TLS modes do not verify the certificate.
        let url = match profile.ssl_mode {
            SslMode::Disable | SslMode::Prefer => format!("redis://{auth}{host}:{port}/{db}"),
            SslMode::Require => format!("rediss://{auth}{host}:{port}/{db}#insecure"),
        };
        let client = redis::Client::open(url)?;
        let conn = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            client.get_connection_manager(),
        )
        .await
        .context("connection timed out")??;
        let d = Self { conn, db };
        d.command(&["PING".into()]).await?;
        Ok(d)
    }

    pub fn db(&self) -> i64 {
        self.db
    }

    async fn raw(&self, args: &[&str]) -> Result<RValue> {
        let mut cmd = redis::cmd(args[0]);
        for a in &args[1..] {
            cmd.arg(*a);
        }
        let mut conn = self.conn.clone();
        Ok(cmd.query_async(&mut conn).await?)
    }

    pub async fn command(&self, args: &[String]) -> Result<Value> {
        if args.is_empty() {
            bail!("empty command");
        }
        if args[0].eq_ignore_ascii_case("select") {
            bail!("use the database menu to switch databases");
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(reply_json(&self.raw(&refs).await?))
    }

    /// Number of databases (CONFIG may be disabled, e.g. on managed Redis).
    pub async fn databases(&self) -> Vec<String> {
        let n = match self.raw(&["CONFIG", "GET", "databases"]).await {
            Ok(v) => pairs(&v)
                .first()
                .and_then(|(_, n)| text(n).parse::<i64>().ok())
                .unwrap_or(16),
            Err(_) => 16,
        };
        (0..n.max(self.db + 1)).map(|i| i.to_string()).collect()
    }

    pub async fn scan(&self, cursor: &str, pattern: &str, count: u32) -> Result<ScanPage> {
        let pattern = if pattern.trim().is_empty() {
            "*"
        } else {
            pattern.trim()
        };
        let count = count.to_string();
        let reply = self
            .raw(&["SCAN", cursor, "MATCH", pattern, "COUNT", &count])
            .await?;
        let RValue::Array(parts) = reply else {
            bail!("unexpected SCAN reply");
        };
        let next = parts.first().map(text).unwrap_or_else(|| "0".into());
        let keys: Vec<String> = match parts.get(1) {
            Some(RValue::Array(k)) => k.iter().map(text).collect(),
            _ => Vec::new(),
        };
        let mut pipe = redis::pipe();
        for k in &keys {
            pipe.cmd("TYPE").arg(k);
        }
        let types: Vec<RValue> = if keys.is_empty() {
            Vec::new()
        } else {
            let mut conn = self.conn.clone();
            pipe.query_async(&mut conn).await?
        };
        let mut keys: Vec<KeyEntry> = keys
            .into_iter()
            .zip(types.iter().map(text))
            .map(|(key, kind)| KeyEntry { key, kind })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(ScanPage { cursor: next, keys })
    }

    pub async fn get(&self, key: &str) -> Result<KeyValue> {
        let kind = text(&self.raw(&["TYPE", key]).await?);
        let ttl = match self.raw(&["TTL", key]).await? {
            RValue::Int(n) => n,
            _ => -1,
        };
        let max = (MAX_ITEMS - 1).to_string();
        let (length, value) = match kind.as_str() {
            "none" => bail!("key {key} does not exist"),
            "string" => {
                let v = self.raw(&["GET", key]).await?;
                let len = match &v {
                    RValue::BulkString(b) => b.len() as u64,
                    _ => 0,
                };
                (len, reply_json(&v))
            }
            "hash" => {
                let len = self.len(&["HLEN", key]).await?;
                let v = if len as usize <= MAX_ITEMS {
                    self.raw(&["HGETALL", key]).await?
                } else {
                    let r = self.raw(&["HSCAN", key, "0", "COUNT", "1000"]).await?;
                    match r {
                        RValue::Array(mut p) if p.len() == 2 => p.remove(1),
                        _ => RValue::Nil,
                    }
                };
                let items: Vec<Value> = pairs(&v)
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|(f, v)| json!([reply_json(f), reply_json(v)]))
                    .collect();
                (len, Value::Array(items))
            }
            "list" => {
                let len = self.len(&["LLEN", key]).await?;
                (
                    len,
                    reply_json(&self.raw(&["LRANGE", key, "0", &max]).await?),
                )
            }
            "set" => {
                let len = self.len(&["SCARD", key]).await?;
                let v = if len as usize <= MAX_ITEMS {
                    self.raw(&["SMEMBERS", key]).await?
                } else {
                    match self.raw(&["SSCAN", key, "0", "COUNT", "1000"]).await? {
                        RValue::Array(mut p) if p.len() == 2 => p.remove(1),
                        _ => RValue::Nil,
                    }
                };
                let mut items: Vec<Value> = match reply_json(&v) {
                    Value::Array(a) => a,
                    _ => Vec::new(),
                };
                items.sort_by_key(|v| v.to_string());
                (len, Value::Array(items))
            }
            "zset" => {
                let len = self.len(&["ZCARD", key]).await?;
                let v = self.raw(&["ZRANGE", key, "0", &max, "WITHSCORES"]).await?;
                let items: Vec<Value> = pairs(&v)
                    .iter()
                    .map(|(m, s)| json!([reply_json(m), text(s)]))
                    .collect();
                (len, Value::Array(items))
            }
            "stream" => {
                let len = self.len(&["XLEN", key]).await?;
                let v = self
                    .raw(&["XRANGE", key, "-", "+", "COUNT", "1000"])
                    .await?;
                let items: Vec<Value> = match &v {
                    RValue::Array(entries) => entries
                        .iter()
                        .filter_map(|e| match e {
                            RValue::Array(p) if p.len() == 2 => {
                                let fields: serde_json::Map<String, Value> = pairs(&p[1])
                                    .iter()
                                    .map(|(f, v)| (text(f), reply_json(v)))
                                    .collect();
                                Some(json!([text(&p[0]), fields]))
                            }
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                (len, Value::Array(items))
            }
            _ => (0, Value::Null),
        };
        let truncated = kind != "string" && (length as usize) > MAX_ITEMS;
        Ok(KeyValue {
            key: key.to_owned(),
            kind,
            ttl,
            length,
            value,
            truncated,
        })
    }

    async fn len(&self, args: &[&str]) -> Result<u64> {
        Ok(match self.raw(args).await? {
            RValue::Int(n) => n.max(0) as u64,
            _ => 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_args() {
        assert_eq!(
            split_args(r#"SET "my key" 'it''s' "a\"b\n" x"#).unwrap(),
            vec!["SET", "my key", "its", "a\"b\n", "x"]
        );
        assert_eq!(split_args(r#"SET k """#).unwrap(), vec!["SET", "k", ""]);
        assert!(split_args(r#"GET "open"#).is_err());
    }

    #[test]
    fn userinfo_is_encoded() {
        assert_eq!(encode("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
    }
}
