use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::mysql::{
    MySqlConnectOptions, MySqlConnection, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode,
};
use sqlx::{Column, ConnectOptions, Connection, Executor, Row, TypeInfo};
use tokio::sync::Mutex;

use super::postgres::ensure_one_row;
use super::sql::{build_change, check_change};
use super::*;
use crate::connections::SslMode;

pub struct MySqlDriver {
    pool: MySqlPool,
    options: MySqlConnectOptions,
    /// Dedicated connection for the SQL editor, with its connection id.
    editor: Mutex<Option<(MySqlConnection, u64)>>,
    running: std::sync::Mutex<HashMap<String, u64>>,
}

/// information_schema types vary across MySQL/MariaDB versions; read as text.
fn text(row: &MySqlRow, i: usize) -> Option<String> {
    row.try_get_unchecked::<Option<String>, _>(i).ok().flatten()
}

const JS_SAFE_INT: i64 = 1 << 53;

impl MySqlDriver {
    pub async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        (host, port): (String, u16),
        database: &str,
    ) -> Result<Self> {
        let mut options = MySqlConnectOptions::new()
            .host(&host)
            .port(if port == 0 { 3306 } else { port })
            .username(&profile.user)
            .password(secrets.password.as_deref().unwrap_or(""))
            .ssl_mode(match profile.ssl_mode {
                SslMode::Disable => MySqlSslMode::Disabled,
                SslMode::Prefer => MySqlSslMode::Preferred,
                SslMode::Require => MySqlSslMode::Required,
            })
            // Leave the server's sql_mode and time zone alone.
            .pipes_as_concat(false)
            .no_engine_substitution(false)
            .timezone(None)
            .disable_statement_logging();
        if !database.is_empty() {
            options = options.database(database);
        }
        let pool = MySqlPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(15))
            .idle_timeout(Duration::from_secs(300))
            .connect_with(options.clone())
            .await?;
        Ok(Self {
            pool,
            options,
            editor: Mutex::new(None),
            running: Default::default(),
        })
    }

    fn columns(row: &MySqlRow) -> Vec<ColumnMeta> {
        row.columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_owned(),
                type_name: c.type_info().name().to_owned(),
            })
            .collect()
    }

    /// Text-protocol rows: every value is its textual form.
    fn row(row: &MySqlRow, cols: &[ColumnMeta]) -> Vec<Value> {
        cols.iter()
            .enumerate()
            .map(|(i, c)| {
                let Ok(Some(bytes)) = row.try_get_unchecked::<Option<&[u8]>, _>(i) else {
                    return Value::Null;
                };
                let t = c.type_name.as_str();
                if t.contains("BLOB") || t.contains("BINARY") || t == "GEOMETRY" {
                    return binary_cell(bytes);
                }
                let s = String::from_utf8_lossy(bytes);
                let base = t.split(' ').next().unwrap_or(t);
                match base {
                    "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "BOOLEAN"
                    | "YEAR" => match s.parse::<i64>() {
                        Ok(n) if n.abs() <= JS_SAFE_INT => Value::from(n),
                        _ => Value::String(s.into_owned()),
                    },
                    "FLOAT" | "DOUBLE" => number_or_string(&s),
                    _ => Value::String(s.into_owned()),
                }
            })
            .collect()
    }

    async fn run_text(
        conn: &mut MySqlConnection,
        sql: &str,
        max_rows: usize,
    ) -> Result<Vec<StatementResult>> {
        let stream = sqlx::raw_sql(sql).fetch_many(&mut *conn);
        let mut out = collect_results(
            stream,
            max_rows,
            |q: &sqlx::mysql::MySqlQueryResult| q.rows_affected(),
            Self::columns,
            Self::row,
        )
        .await?;
        if let [only] = out.as_mut_slice() {
            if only.result.is_none() && returns_rows(sql) {
                if let Ok(d) = conn.describe(sql).await {
                    only.result = Some(ResultSet {
                        columns: d
                            .columns()
                            .iter()
                            .map(|c| ColumnMeta {
                                name: c.name().to_owned(),
                                type_name: c.type_info().name().to_owned(),
                            })
                            .collect(),
                        ..Default::default()
                    });
                }
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Driver for MySqlDriver {
    fn dialect(&self) -> Dialect {
        Dialect::Mysql
    }

    async fn current_database(&self) -> Result<String> {
        let row = sqlx::query("SELECT DATABASE()")
            .fetch_one(&self.pool)
            .await?;
        Ok(text(&row, 0).unwrap_or_default())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        self.list_schemas().await
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT CAST(SCHEMA_NAME AS CHAR) FROM information_schema.SCHEMATA ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().filter_map(|r| text(r, 0)).collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows = sqlx::query(
            "SELECT CAST(TABLE_NAME AS CHAR), CAST(TABLE_TYPE AS CHAR) FROM information_schema.TABLES
             WHERE TABLE_SCHEMA = ? ORDER BY 1",
        )
        .bind(schema)
        .fetch_all(&self.pool)
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
        let cols = sqlx::query(
            "SELECT CAST(COLUMN_NAME AS CHAR), CAST(COLUMN_TYPE AS CHAR), CAST(IS_NULLABLE AS CHAR),
                    CAST(COLUMN_DEFAULT AS CHAR), CAST(COLUMN_KEY AS CHAR), CAST(EXTRA AS CHAR)
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION",
        )
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;
        if cols.is_empty() {
            anyhow::bail!("table {}.{} not found", t.schema, t.name);
        }
        let stats = sqlx::query(
            "SELECT CAST(INDEX_NAME AS CHAR), CAST(NON_UNIQUE AS CHAR), CAST(COLUMN_NAME AS CHAR)
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;
        let fk_rows = sqlx::query(
            "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR),
                    CAST(REFERENCED_TABLE_SCHEMA AS CHAR), CAST(REFERENCED_TABLE_NAME AS CHAR),
                    CAST(REFERENCED_COLUMN_NAME AS CHAR)
             FROM information_schema.KEY_COLUMN_USAGE
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL
             ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION",
        )
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;

        let mut indexes: Vec<IndexInfo> = Vec::new();
        for r in &stats {
            let name = text(r, 0).unwrap_or_default();
            let col = text(r, 2).unwrap_or_else(|| "expr".into());
            match indexes.last_mut() {
                Some(ix) if ix.name == name => ix.columns.push(col),
                _ => indexes.push(IndexInfo {
                    primary: name == "PRIMARY",
                    unique: text(r, 1).as_deref() == Some("0"),
                    name,
                    columns: vec![col],
                }),
            }
        }
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        for r in &fk_rows {
            let name = text(r, 0).unwrap_or_default();
            let col = text(r, 1).unwrap_or_default();
            let ref_col = text(r, 4).unwrap_or_default();
            match foreign_keys.last_mut() {
                Some(fk) if fk.name == name => {
                    fk.columns.push(col);
                    fk.ref_columns.push(ref_col);
                }
                _ => foreign_keys.push(ForeignKeyInfo {
                    name,
                    columns: vec![col],
                    ref_table: format!(
                        "{}.{}",
                        text(r, 2).unwrap_or_default(),
                        text(r, 3).unwrap_or_default()
                    ),
                    ref_columns: vec![ref_col],
                }),
            }
        }
        Ok(TableStructure {
            columns: cols
                .iter()
                .map(|r| {
                    let extra = text(r, 5).unwrap_or_default();
                    ColumnInfo {
                        name: text(r, 0).unwrap_or_default(),
                        data_type: text(r, 1).unwrap_or_default(),
                        nullable: text(r, 2).as_deref() == Some("YES"),
                        default: text(r, 3).or(if extra.contains("auto_increment") {
                            Some("auto_increment".into())
                        } else {
                            None
                        }),
                        primary_key: text(r, 4).as_deref() == Some("PRI"),
                    }
                })
                .collect(),
            indexes,
            foreign_keys,
        })
    }

    async fn execute(&self, sql: &str, max_rows: usize, query_id: &str) -> Result<ExecuteResult> {
        let mut guard = self.editor.lock().await;
        if guard.is_none() {
            let mut conn = self.options.connect().await?;
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut conn)
                .await?;
            *guard = Some((conn, id));
        }
        let (conn, id) = guard.as_mut().expect("editor connection was just set");
        self.running
            .lock()
            .unwrap()
            .insert(query_id.to_owned(), *id);
        let start = Instant::now();
        let res = Self::run_text(conn, sql, max_rows).await;
        self.running.lock().unwrap().remove(query_id);
        if let Err(e) = &res {
            if matches!(
                e.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::Io(_) | sqlx::Error::Protocol(_))
            ) {
                *guard = None;
            }
        }
        Ok(ExecuteResult {
            statements: res?,
            duration_ms: elapsed_ms(start),
        })
    }

    async fn cancel(&self, query_id: &str) -> Result<()> {
        let id = self.running.lock().unwrap().get(query_id).copied();
        if let Some(id) = id {
            sqlx::raw_sql(&format!("KILL QUERY {id}"))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    async fn query(&self, sql: &str, max_rows: usize) -> Result<ResultSet> {
        let mut conn = self.pool.acquire().await?;
        let out = Self::run_text(&mut conn, sql, max_rows).await?;
        Ok(out.into_iter().find_map(|s| s.result).unwrap_or_default())
    }

    async fn apply_changes(&self, table: &TableRef, changes: &[RowChange]) -> Result<u64> {
        let s = self.table_structure(table).await?;
        let mut tx = self.pool.begin().await?;
        let mut total = 0;
        for change in changes {
            check_change(&s, change)?;
            let (sql, params) = build_change(Dialect::Mysql, table, change, &s, false);
            let mut q = sqlx::query(&sql);
            for p in params {
                q = q.bind(p);
            }
            let n = q
                .execute(&mut *tx)
                .await
                .with_context(|| sql.clone())?
                .rows_affected();
            ensure_one_row(change, n)?;
            total += n;
        }
        tx.commit().await?;
        Ok(total)
    }

    async fn close(&self) {
        if let Some((conn, _)) = self.editor.lock().await.take() {
            let _ = conn.close().await;
        }
        self.pool.close().await;
    }
}
