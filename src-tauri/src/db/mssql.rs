//! SQL Server through tiberius. tiberius has no pool, so the driver keeps one
//! connection for metadata, browsing and edits, and one for the SQL editor.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use serde_json::Value;
use tiberius::{
    AuthMethod, Client, ColumnData, Config, EncryptionLevel, FromSql, QueryItem, Row, ToSql,
};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::postgres::ensure_one_row;
use super::sql::{build_change, check_change};
use super::*;
use crate::connections::SslMode;

type Conn = Client<Compat<TcpStream>>;

pub struct MssqlDriver {
    config: Config,
    meta: Mutex<Conn>,
    /// Dedicated editor connection with its session id (@@SPID).
    editor: Mutex<Option<(Conn, i16)>>,
    running: std::sync::Mutex<HashMap<String, i16>>,
    cancelled: std::sync::Mutex<HashSet<String>>,
}

async fn open(config: &Config) -> Result<Conn> {
    let tcp = tokio::time::timeout(
        Duration::from_secs(15),
        TcpStream::connect(config.get_addr()),
    )
    .await
    .context("connection timed out")??;
    tcp.set_nodelay(true)?;
    Ok(Client::connect(config.clone(), tcp.compat_write()).await?)
}

fn is_io(e: &tiberius::error::Error) -> bool {
    matches!(e, tiberius::error::Error::Io { .. })
}

const JS_SAFE_INT: i64 = 1 << 53;

fn cell(data: &ColumnData<'static>) -> Value {
    let dt = |v: Option<chrono::NaiveDateTime>| {
        v.map_or(Value::Null, |v| {
            Value::String(v.format("%Y-%m-%d %H:%M:%S%.f").to_string())
        })
    };
    match data {
        ColumnData::U8(v) => v.map_or(Value::Null, Value::from),
        ColumnData::I16(v) => v.map_or(Value::Null, Value::from),
        ColumnData::I32(v) => v.map_or(Value::Null, Value::from),
        ColumnData::I64(v) => v.map_or(Value::Null, |n| {
            if n.abs() <= JS_SAFE_INT {
                Value::from(n)
            } else {
                Value::String(n.to_string())
            }
        }),
        ColumnData::F32(v) => v.map_or(Value::Null, |f| number_or_string(&f.to_string())),
        ColumnData::F64(v) => v.map_or(Value::Null, |f| number_or_string(&f.to_string())),
        ColumnData::Bit(v) => v.map_or(Value::Null, Value::Bool),
        ColumnData::String(v) => v
            .as_ref()
            .map_or(Value::Null, |s| Value::String(s.to_string())),
        ColumnData::Guid(v) => {
            v.map_or(Value::Null, |g| Value::String(g.to_string().to_uppercase()))
        }
        ColumnData::Binary(v) => v.as_ref().map_or(Value::Null, |b| binary_cell(b)),
        ColumnData::Numeric(v) => v.map_or(Value::Null, |n| Value::String(n.to_string())),
        ColumnData::Xml(v) => v
            .as_ref()
            .map_or(Value::Null, |x| Value::String(x.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            dt(chrono::NaiveDateTime::from_sql(data).ok().flatten())
        }
        ColumnData::Date(_) => chrono::NaiveDate::from_sql(data)
            .ok()
            .flatten()
            .map_or(Value::Null, |d| Value::String(d.to_string())),
        ColumnData::Time(_) => chrono::NaiveTime::from_sql(data)
            .ok()
            .flatten()
            .map_or(Value::Null, |t| {
                Value::String(t.format("%H:%M:%S%.f").to_string())
            }),
        ColumnData::DateTimeOffset(_) => chrono::DateTime::<chrono::FixedOffset>::from_sql(data)
            .ok()
            .flatten()
            .map_or(Value::Null, |d| {
                Value::String(d.format("%Y-%m-%d %H:%M:%S%.f %:z").to_string())
            }),
    }
}

fn text(row: &Row, i: usize) -> Option<String> {
    let (_, data) = row.cells().nth(i)?;
    match cell(data) {
        Value::Null => None,
        Value::String(s) => Some(s),
        v => Some(v.to_string()),
    }
}

fn type_name(t: tiberius::ColumnType) -> String {
    use tiberius::ColumnType as T;
    match t {
        T::Int1 => "tinyint",
        T::Int2 => "smallint",
        T::Int4 | T::Intn => "int",
        T::Int8 => "bigint",
        T::Bit | T::Bitn => "bit",
        T::Float4 => "real",
        T::Float8 | T::Floatn => "float",
        T::Decimaln | T::Numericn => "decimal",
        T::Money | T::Money4 => "money",
        T::Datetime | T::Datetimen | T::Datetime4 => "datetime",
        T::Datetime2 => "datetime2",
        T::DatetimeOffsetn => "datetimeoffset",
        T::Daten => "date",
        T::Timen => "time",
        T::Guid => "uniqueidentifier",
        T::BigVarChar | T::BigChar => "varchar",
        T::NVarchar | T::NChar => "nvarchar",
        T::Text => "text",
        T::NText => "ntext",
        T::BigVarBin | T::BigBinary => "varbinary",
        T::Image => "image",
        T::Xml => "xml",
        other => return format!("{other:?}").to_lowercase(),
    }
    .to_owned()
}

fn columns_meta(cols: &[tiberius::Column]) -> Vec<ColumnMeta> {
    cols.iter()
        .map(|c| ColumnMeta {
            name: c.name().to_owned(),
            type_name: type_name(c.column_type()),
        })
        .collect()
}

/// Batches are separated by `GO` lines, as in SSMS.
fn batches(sql: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in sql.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            out.push(String::new());
        } else {
            let cur = out.last_mut().expect("never empty");
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out.into_iter().filter(|b| !b.trim().is_empty()).collect()
}

/// tiberius returns either rows or affected-row counts for a batch, not both.
/// Data-changing batches use the counts; everything else the rows.
fn wants_rows(batch: &str) -> bool {
    let first = batch
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match first.as_str() {
        "insert" | "update" | "delete" | "merge" | "create" | "alter" | "drop" | "truncate" => {
            false
        }
        _ => {
            returns_rows(batch)
                || matches!(first.as_str(), "exec" | "execute")
                || batch.to_ascii_lowercase().contains("select")
        }
    }
}

/// DDL such as CREATE VIEW must run as a plain batch, not via sp_executesql.
fn is_ddl(batch: &str) -> bool {
    let first = batch
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(first.as_str(), "create" | "alter" | "drop" | "truncate")
}

async fn run_batch(conn: &mut Conn, batch: &str, max_rows: usize) -> Result<Vec<StatementResult>> {
    if is_ddl(batch) {
        conn.simple_query(batch).await?.into_results().await?;
        return Ok(vec![StatementResult {
            result: None,
            rows_affected: 0,
        }]);
    }
    if !wants_rows(batch) {
        let res = conn.execute(batch, &[]).await?;
        let counts = res.rows_affected();
        return Ok(if counts.is_empty() {
            vec![StatementResult {
                result: None,
                rows_affected: 0,
            }]
        } else {
            counts
                .iter()
                .map(|&n| StatementResult {
                    result: None,
                    rows_affected: n,
                })
                .collect()
        });
    }
    let mut stream = conn.simple_query(batch).await?;
    let mut out: Vec<StatementResult> = Vec::new();
    // tiberius needs the stream drained, so rows past the limit are skipped.
    while let Some(item) = stream.try_next().await? {
        match item {
            QueryItem::Metadata(m) => out.push(StatementResult {
                result: Some(ResultSet {
                    columns: columns_meta(m.columns()),
                    ..Default::default()
                }),
                rows_affected: 0,
            }),
            QueryItem::Row(row) => {
                let Some(rs) = out.last_mut().and_then(|s| s.result.as_mut()) else {
                    continue;
                };
                if rs.rows.len() >= max_rows {
                    rs.truncated = true;
                    continue;
                }
                rs.rows.push(row.cells().map(|(_, d)| cell(d)).collect());
            }
        }
    }
    if out.is_empty() {
        out.push(StatementResult {
            result: None,
            rows_affected: 0,
        });
    }
    Ok(out)
}

impl MssqlDriver {
    pub async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        (host, port): (String, u16),
        database: &str,
    ) -> Result<Self> {
        let mut config = Config::new();
        config.host(&host);
        config.port(if port == 0 { 1433 } else { port });
        config.authentication(AuthMethod::sql_server(
            &profile.user,
            secrets.password.as_deref().unwrap_or(""),
        ));
        config.application_name("Tablory");
        if !database.is_empty() {
            config.database(database);
        }
        // Like the other drivers, SSL modes encrypt without verifying the
        // certificate (SQL Server ships a self-signed one by default).
        match profile.ssl_mode {
            SslMode::Disable => config.encryption(EncryptionLevel::NotSupported),
            SslMode::Prefer => {
                config.encryption(EncryptionLevel::On);
                config.trust_cert();
            }
            SslMode::Require => {
                config.encryption(EncryptionLevel::Required);
                config.trust_cert();
            }
        }
        let meta = open(&config).await?;
        Ok(Self {
            config,
            meta: Mutex::new(meta),
            editor: Mutex::new(None),
            running: Default::default(),
            cancelled: Default::default(),
        })
    }

    /// Runs a parameterized query on the metadata connection, reconnecting
    /// once if the connection dropped.
    async fn rows(&self, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Row>> {
        let mut c = self.meta.lock().await;
        let first = match c.query(sql, params).await {
            Ok(s) => s.into_first_result().await,
            Err(e) => Err(e),
        };
        match first {
            Err(e) if is_io(&e) => {
                *c = open(&self.config).await?;
                Ok(c.query(sql, params).await?.into_first_result().await?)
            }
            r => Ok(r?),
        }
    }
}

#[async_trait]
impl Driver for MssqlDriver {
    fn dialect(&self) -> Dialect {
        Dialect::Mssql
    }

    async fn current_database(&self) -> Result<String> {
        let rows = self.rows("SELECT DB_NAME()", &[]).await?;
        Ok(rows.first().and_then(|r| text(r, 0)).unwrap_or_default())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let rows = self
            .rows(
                "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 AND state = 0 ORDER BY name",
                &[],
            )
            .await?;
        Ok(rows.iter().filter_map(|r| text(r, 0)).collect())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let rows = self
            .rows(
                "SELECT s.name FROM sys.schemas s
                 WHERE s.name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') AND s.name NOT LIKE 'db[_]%'
                 ORDER BY CASE WHEN s.name = 'dbo' THEN 0 ELSE 1 END, s.name",
                &[],
            )
            .await?;
        Ok(rows.iter().filter_map(|r| text(r, 0)).collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows = self
            .rows(
                "SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES
                 WHERE TABLE_SCHEMA = @P1 ORDER BY TABLE_NAME",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| TableInfo {
                name: text(r, 0).unwrap_or_default(),
                kind: if text(r, 1).unwrap_or_default().contains("VIEW") {
                    "view".into()
                } else {
                    "table".into()
                },
            })
            .collect())
    }

    async fn table_structure(&self, t: &TableRef) -> Result<TableStructure> {
        let schema = t.schema.as_str();
        let name = t.name.as_str();
        let cols = self
            .rows(
                "SELECT c.COLUMN_NAME,
                    c.DATA_TYPE + CASE
                      WHEN c.DATA_TYPE IN ('varchar', 'nvarchar', 'char', 'nchar', 'varbinary', 'binary')
                        THEN '(' + CASE WHEN c.CHARACTER_MAXIMUM_LENGTH = -1 THEN 'max'
                                        ELSE CAST(c.CHARACTER_MAXIMUM_LENGTH AS varchar(10)) END + ')'
                      WHEN c.DATA_TYPE IN ('decimal', 'numeric')
                        THEN '(' + CAST(c.NUMERIC_PRECISION AS varchar(10)) + ',' + CAST(c.NUMERIC_SCALE AS varchar(10)) + ')'
                      WHEN c.DATA_TYPE IN ('datetime2', 'time', 'datetimeoffset')
                        THEN '(' + CAST(c.DATETIME_PRECISION AS varchar(10)) + ')'
                      ELSE '' END,
                    c.IS_NULLABLE, c.COLUMN_DEFAULT,
                    CASE WHEN EXISTS (
                      SELECT 1 FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                      JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE k
                        ON k.CONSTRAINT_NAME = tc.CONSTRAINT_NAME AND k.TABLE_SCHEMA = tc.TABLE_SCHEMA
                      WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' AND tc.TABLE_SCHEMA = c.TABLE_SCHEMA
                        AND tc.TABLE_NAME = c.TABLE_NAME AND k.COLUMN_NAME = c.COLUMN_NAME) THEN 1 ELSE 0 END,
                    COLUMNPROPERTY(OBJECT_ID(QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)), c.COLUMN_NAME, 'IsIdentity')
                 FROM INFORMATION_SCHEMA.COLUMNS c
                 WHERE c.TABLE_SCHEMA = @P1 AND c.TABLE_NAME = @P2
                 ORDER BY c.ORDINAL_POSITION",
                &[&schema, &name],
            )
            .await?;
        if cols.is_empty() {
            bail!("table {schema}.{name} not found");
        }
        let ix_rows = self
            .rows(
                "SELECT i.name, i.is_unique, i.is_primary_key, c.name
                 FROM sys.indexes i
                 JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
                 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
                 WHERE i.object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))
                   AND i.type > 0 AND ic.is_included_column = 0
                 ORDER BY i.is_primary_key DESC, i.name, ic.key_ordinal",
                &[&schema, &name],
            )
            .await?;
        let fk_rows = self
            .rows(
                "SELECT fk.name, pc.name,
                        OBJECT_SCHEMA_NAME(fk.referenced_object_id) + '.' + OBJECT_NAME(fk.referenced_object_id),
                        rc.name
                 FROM sys.foreign_keys fk
                 JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
                 JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
                 JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
                 WHERE fk.parent_object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))
                 ORDER BY fk.name, fkc.constraint_column_id",
                &[&schema, &name],
            )
            .await?;

        let mut indexes: Vec<IndexInfo> = Vec::new();
        for r in &ix_rows {
            let ix = text(r, 0).unwrap_or_default();
            let col = text(r, 3).unwrap_or_default();
            match indexes.last_mut() {
                Some(last) if last.name == ix => last.columns.push(col),
                _ => indexes.push(IndexInfo {
                    name: ix,
                    columns: vec![col],
                    unique: text(r, 1).as_deref() == Some("true"),
                    primary: text(r, 2).as_deref() == Some("true"),
                }),
            }
        }
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        for r in &fk_rows {
            let fk = text(r, 0).unwrap_or_default();
            let col = text(r, 1).unwrap_or_default();
            let ref_col = text(r, 3).unwrap_or_default();
            match foreign_keys.last_mut() {
                Some(last) if last.name == fk => {
                    last.columns.push(col);
                    last.ref_columns.push(ref_col);
                }
                _ => foreign_keys.push(ForeignKeyInfo {
                    name: fk,
                    columns: vec![col],
                    ref_table: text(r, 2).unwrap_or_default(),
                    ref_columns: vec![ref_col],
                }),
            }
        }
        Ok(TableStructure {
            columns: cols
                .iter()
                .map(|r| ColumnInfo {
                    name: text(r, 0).unwrap_or_default(),
                    data_type: text(r, 1).unwrap_or_default(),
                    nullable: text(r, 2).as_deref() == Some("YES"),
                    default: text(r, 3).or_else(|| {
                        (text(r, 5).as_deref() == Some("1")).then(|| "identity".into())
                    }),
                    primary_key: text(r, 4).as_deref() == Some("1"),
                })
                .collect(),
            indexes,
            foreign_keys,
        })
    }

    async fn execute(&self, sql: &str, max_rows: usize, query_id: &str) -> Result<ExecuteResult> {
        let mut guard = self.editor.lock().await;
        if guard.is_none() {
            let mut conn = open(&self.config).await?;
            let spid: i16 = conn
                .simple_query("SELECT @@SPID")
                .await?
                .into_row()
                .await?
                .and_then(|r| r.get(0))
                .context("could not read @@SPID")?;
            *guard = Some((conn, spid));
        }
        let (conn, spid) = guard.as_mut().expect("editor connection was just set");
        self.running
            .lock()
            .unwrap()
            .insert(query_id.to_owned(), *spid);
        let start = Instant::now();
        let mut statements = Vec::new();
        let mut res = Ok(());
        for batch in batches(sql) {
            match run_batch(conn, &batch, max_rows).await {
                Ok(mut s) => statements.append(&mut s),
                Err(e) => {
                    res = Err(e);
                    break;
                }
            }
        }
        self.running.lock().unwrap().remove(query_id);
        let was_cancelled = self.cancelled.lock().unwrap().remove(query_id);
        if let Err(e) = res {
            // KILL ends the whole session, and an I/O error leaves nothing to reuse.
            let broken = was_cancelled
                || e.downcast_ref::<tiberius::error::Error>()
                    .is_some_and(is_io);
            if broken {
                *guard = None;
            }
            if was_cancelled {
                bail!("query cancelled");
            }
            return Err(e);
        }
        Ok(ExecuteResult {
            statements,
            duration_ms: elapsed_ms(start),
        })
    }

    /// SQL Server can only cancel by killing the session; the editor
    /// reconnects on its next run.
    async fn cancel(&self, query_id: &str) -> Result<()> {
        let spid = self.running.lock().unwrap().get(query_id).copied();
        if let Some(spid) = spid {
            self.cancelled.lock().unwrap().insert(query_id.to_owned());
            let mut c = self.meta.lock().await;
            c.execute(format!("KILL {spid}"), &[]).await?;
        }
        Ok(())
    }

    async fn query(&self, sql: &str, max_rows: usize) -> Result<ResultSet> {
        let mut c = self.meta.lock().await;
        let out = match run_batch(&mut c, sql, max_rows).await {
            Err(e)
                if e.downcast_ref::<tiberius::error::Error>()
                    .is_some_and(is_io) =>
            {
                *c = open(&self.config).await?;
                run_batch(&mut c, sql, max_rows).await?
            }
            r => r?,
        };
        Ok(out.into_iter().find_map(|s| s.result).unwrap_or_default())
    }

    async fn apply_changes(&self, table: &TableRef, changes: &[RowChange]) -> Result<u64> {
        let s = self.table_structure(table).await?;
        for change in changes {
            check_change(&s, change)?;
        }
        let mut c = self.meta.lock().await;
        c.simple_query("BEGIN TRANSACTION")
            .await?
            .into_results()
            .await?;
        let mut total = 0;
        let mut outcome = Ok(());
        for change in changes {
            let (sql, params) = build_change(Dialect::Mssql, table, change, &s, false);
            let refs: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
            let step = match c.execute(sql.as_str(), &refs).await {
                Ok(r) => {
                    let n = r.total();
                    ensure_one_row(change, n).map(|_| n)
                }
                Err(e) => Err(anyhow::Error::new(e).context(sql.clone())),
            };
            match step {
                Ok(n) => total += n,
                Err(e) => {
                    outcome = Err(e);
                    break;
                }
            }
        }
        match outcome {
            Ok(()) => {
                c.simple_query("COMMIT").await?.into_results().await?;
                Ok(total)
            }
            Err(e) => {
                if let Ok(s) = c.simple_query("IF @@TRANCOUNT > 0 ROLLBACK").await {
                    let _ = s.into_results().await;
                }
                Err(e)
            }
        }
    }

    async fn close(&self) {
        if let Some((conn, _)) = self.editor.lock().await.take() {
            let _ = conn.close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_go_batches() {
        let b = batches("select 1\nGO\n  go  \nupdate t set a = 1\nselect 'go'\n");
        assert_eq!(b, vec!["select 1\n", "update t set a = 1\nselect 'go'\n"]);
    }

    #[test]
    fn routes_batches() {
        assert!(wants_rows("SELECT 1"));
        assert!(wants_rows("DECLARE @x int = 1; SELECT @x"));
        assert!(wants_rows("EXEC sp_who"));
        assert!(!wants_rows("UPDATE t SET a = 1"));
        assert!(!wants_rows("INSERT INTO t SELECT * FROM u"));
        assert!(!wants_rows("CREATE TABLE t (id int)"));
    }
}
