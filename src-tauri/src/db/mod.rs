//! Database drivers. Every SQL engine implements [`Driver`]; the frontend only
//! ever sees the serde types defined here.

pub mod mssql;
pub mod mysql;
pub mod postgres;
pub mod sql;
pub mod sqlite;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::{Stream, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Either;

use crate::connections::{ConnectionProfile, DbKind, Secrets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    Postgres,
    Mysql,
    Sqlite,
    Mssql,
}

#[derive(Serialize, Debug)]
pub struct TableInfo {
    pub name: String,
    /// "table" or "view".
    pub kind: String,
}

#[derive(Serialize, Debug)]
pub struct RoutineInfo {
    pub name: String,
    /// "function" or "procedure".
    pub kind: String,
    /// What [`Driver::routine_definition`] looks it up by: the object id on
    /// PostgreSQL (overloads share a name) and SQL Server, else the name.
    pub id: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TableRef {
    pub schema: String,
    pub name: String,
}

#[derive(Serialize, Debug)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Serialize, Debug)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Serialize, Debug)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct TableStructure {
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

impl TableStructure {
    pub fn primary_key(&self) -> Vec<&str> {
        self.columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect()
    }

    pub fn column_type(&self, name: &str) -> Option<&str> {
        self.columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.data_type.as_str())
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

/// Cells are plain JSON: null, bool, number or string. Values that would lose
/// precision as a JS number (int8, numeric) stay strings; binary data becomes
/// `{"$bin": <byte length>, "hex": <preview>}`.
#[derive(Serialize, Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
    /// More rows existed but were not fetched.
    pub truncated: bool,
}

#[derive(Serialize, Debug)]
pub struct StatementResult {
    pub result: Option<ResultSet>,
    pub rows_affected: u64,
}

#[derive(Serialize, Debug)]
pub struct ExecuteResult {
    pub statements: Vec<StatementResult>,
    pub duration_ms: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Filter {
    pub column: String,
    pub op: String,
    pub value: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Sort {
    pub column: String,
    pub desc: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RowsRequest {
    pub table: TableRef,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Free-form WHERE clause typed by the user.
    #[serde(default)]
    pub raw_where: Option<String>,
    #[serde(default)]
    pub sort: Option<Sort>,
    pub limit: u32,
    pub offset: u64,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ColValue {
    pub column: String,
    /// Text form of the value; `None` is SQL NULL.
    pub value: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RowChange {
    Update {
        key: Vec<ColValue>,
        values: Vec<ColValue>,
    },
    Insert {
        values: Vec<ColValue>,
    },
    Delete {
        key: Vec<ColValue>,
    },
}

#[async_trait]
pub trait Driver: Send + Sync {
    fn dialect(&self) -> Dialect;
    async fn current_database(&self) -> Result<String>;
    async fn list_databases(&self) -> Result<Vec<String>>;
    async fn list_schemas(&self) -> Result<Vec<String>>;
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>>;
    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure>;
    /// Runs user SQL (possibly several statements) on the session's dedicated
    /// connection, so BEGIN/COMMIT across runs behave as expected.
    async fn execute(&self, sql: &str, max_rows: usize, query_id: &str) -> Result<ExecuteResult>;
    async fn cancel(&self, query_id: &str) -> Result<()>;
    /// Runs one read-only statement on the pool.
    async fn query(&self, sql: &str, max_rows: usize) -> Result<ResultSet>;
    async fn apply_changes(&self, table: &TableRef, changes: &[RowChange]) -> Result<u64>;
    async fn close(&self);

    /// User-defined functions and procedures in the schema.
    async fn list_routines(&self, schema: &str) -> Result<Vec<RoutineInfo>> {
        let d = self.dialect();
        let s = sql::quote_literal(d, schema);
        let q = match d {
            // Skips aggregates and functions that belong to an extension.
            Dialect::Postgres => format!(
                "SELECT p.proname::text,
                        CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END,
                        p.oid::text
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                 WHERE n.nspname = {s} AND p.prokind IN ('f', 'p')
                   AND NOT EXISTS (SELECT 1 FROM pg_depend e
                                   WHERE e.classid = 'pg_proc'::regclass
                                     AND e.objid = p.oid AND e.deptype = 'e')
                 ORDER BY 1"
            ),
            Dialect::Mysql => format!(
                "SELECT CAST(ROUTINE_NAME AS CHAR), CAST(LOWER(ROUTINE_TYPE) AS CHAR),
                        CAST(ROUTINE_NAME AS CHAR)
                 FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = {s} ORDER BY 1"
            ),
            Dialect::Mssql => format!(
                "SELECT o.name,
                        CASE WHEN o.type IN ('P', 'PC') THEN 'procedure' ELSE 'function' END,
                        CAST(o.object_id AS varchar(20))
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
                 WHERE s.name = {s} AND o.is_ms_shipped = 0
                   AND o.type IN ('P', 'PC', 'FN', 'IF', 'TF', 'FS', 'FT')
                 ORDER BY o.name"
            ),
            Dialect::Sqlite => return Ok(Vec::new()),
        };
        let rs = self.query(&q, 100_000).await?;
        let text = |v: Option<&Value>| match v {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        };
        Ok(rs
            .rows
            .iter()
            .map(|r| RoutineInfo {
                name: text(r.first()),
                kind: text(r.get(1)),
                id: text(r.get(2)),
            })
            .collect())
    }

    /// CREATE statement of a routine from [`Driver::list_routines`].
    async fn routine_definition(&self, schema: &str, kind: &str, id: &str) -> Result<String> {
        let d = self.dialect();
        let (q, col) = match d {
            Dialect::Postgres => {
                let oid: u32 = id.parse()?;
                (format!("SELECT pg_get_functiondef({oid})"), 0)
            }
            Dialect::Mysql => {
                let what = if kind == "procedure" {
                    "PROCEDURE"
                } else {
                    "FUNCTION"
                };
                let name = sql::qualified(
                    d,
                    &TableRef {
                        schema: schema.to_owned(),
                        name: id.to_owned(),
                    },
                );
                // Columns: name, sql_mode, "Create Function|Procedure", ...
                (format!("SHOW CREATE {what} {name}"), 2)
            }
            Dialect::Mssql => {
                let oid: i64 = id.parse()?;
                (format!("SELECT OBJECT_DEFINITION({oid})"), 0)
            }
            Dialect::Sqlite => anyhow::bail!("SQLite has no stored routines"),
        };
        let rs = self.query(&q, 1).await?;
        match rs.rows.first().and_then(|r| r.get(col)) {
            Some(Value::String(s)) => Ok(s.clone()),
            _ => anyhow::bail!("the definition is not visible (missing permission?)"),
        }
    }

    async fn fetch_rows(&self, req: &RowsRequest) -> Result<ResultSet> {
        let sql = sql::build_select(self.dialect(), req);
        let mut rs = self.query(&sql, req.limit as usize).await?;
        if rs.columns.is_empty() {
            // No rows came back, so the driver could not see the columns.
            let s = self.table_structure(&req.table).await?;
            rs.columns = s
                .columns
                .into_iter()
                .map(|c| ColumnMeta {
                    name: c.name,
                    type_name: c.data_type,
                })
                .collect();
        }
        Ok(rs)
    }

    async fn count_rows(&self, req: &RowsRequest) -> Result<u64> {
        let sql = sql::build_count(self.dialect(), req);
        let rs = self.query(&sql, 1).await?;
        let v = rs.rows.first().and_then(|r| r.first());
        Ok(match v {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        })
    }

    async fn preview_changes(&self, table: &TableRef, changes: &[RowChange]) -> Result<String> {
        let s = self.table_structure(table).await?;
        Ok(changes
            .iter()
            .map(|c| sql::build_change(self.dialect(), table, c, &s, true).0 + ";")
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// Builds a driver for the profile. `endpoint` overrides host/port, e.g. with
/// the local end of an SSH tunnel.
pub async fn connect(
    profile: &ConnectionProfile,
    secrets: &Secrets,
    endpoint: (String, u16),
    database: &str,
) -> Result<Arc<dyn Driver>> {
    Ok(match profile.kind {
        DbKind::Postgres => {
            Arc::new(postgres::PgDriver::connect(profile, secrets, endpoint, database).await?)
        }
        DbKind::Mysql => {
            Arc::new(mysql::MySqlDriver::connect(profile, secrets, endpoint, database).await?)
        }
        DbKind::Sqlite => Arc::new(sqlite::SqliteDriver::connect(profile).await?),
        DbKind::Mssql => {
            Arc::new(mssql::MssqlDriver::connect(profile, secrets, endpoint, database).await?)
        }
        other => anyhow::bail!("{other:?} is not supported yet"),
    })
}

/// Groups a raw multi-statement stream into per-statement results.
pub(crate) async fn collect_results<S, Q, R>(
    mut stream: S,
    max_rows: usize,
    affected: impl Fn(&Q) -> u64,
    columns: impl Fn(&R) -> Vec<ColumnMeta>,
    row: impl Fn(&R, &[ColumnMeta]) -> Vec<Value>,
) -> Result<Vec<StatementResult>>
where
    S: Stream<Item = Result<Either<Q, R>, sqlx::Error>> + Unpin,
{
    let mut out = Vec::new();
    let mut current: Option<ResultSet> = None;
    while let Some(item) = stream.try_next().await? {
        match item {
            Either::Right(r) => {
                let rs = current.get_or_insert_with(|| ResultSet {
                    columns: columns(&r),
                    ..Default::default()
                });
                if rs.rows.len() >= max_rows {
                    rs.truncated = true;
                    break;
                }
                let cells = row(&r, &rs.columns);
                rs.rows.push(cells);
            }
            Either::Left(q) => out.push(StatementResult {
                result: current.take(),
                rows_affected: affected(&q),
            }),
        }
    }
    if let Some(rs) = current.take() {
        out.push(StatementResult {
            result: Some(rs),
            rows_affected: 0,
        });
    }
    Ok(out)
}

/// True for statements that return rows, so an empty result still gets headers.
pub(crate) fn returns_rows(sql: &str) -> bool {
    let first = sql
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        first.as_str(),
        "select"
            | "with"
            | "table"
            | "values"
            | "show"
            | "explain"
            | "pragma"
            | "describe"
            | "desc"
    )
}

pub(crate) fn binary_cell(bytes: &[u8]) -> Value {
    let hex: String = bytes.iter().take(64).map(|b| format!("{b:02x}")).collect();
    serde_json::json!({ "$bin": bytes.len(), "hex": hex })
}

pub(crate) fn number_or_string(s: &str) -> Value {
    s.parse::<serde_json::Number>()
        .ok()
        .filter(|n| n.as_f64().is_some_and(f64::is_finite))
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(s.to_owned()))
}

pub(crate) fn elapsed_ms(start: std::time::Instant) -> u64 {
    start.elapsed().as_millis() as u64
}
