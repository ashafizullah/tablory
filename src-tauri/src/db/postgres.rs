use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions, PgRow, PgSslMode};
use sqlx::{Column, ConnectOptions, Connection, Executor, Row, TypeInfo};
use tokio::sync::Mutex;

use super::sql::{build_change, check_change};
use super::*;
use crate::connections::SslMode;

pub struct PgDriver {
    pool: PgPool,
    options: PgConnectOptions,
    /// Dedicated connection for the SQL editor, with its backend pid.
    editor: Mutex<Option<(PgConnection, i32)>>,
    running: std::sync::Mutex<HashMap<String, i32>>,
}

impl PgDriver {
    pub async fn connect(
        profile: &ConnectionProfile,
        secrets: &Secrets,
        (host, port): (String, u16),
        database: &str,
    ) -> Result<Self> {
        let database = if database.is_empty() {
            "postgres"
        } else {
            database
        };
        let options = PgConnectOptions::new()
            .host(&host)
            .port(if port == 0 { 5432 } else { port })
            .username(&profile.user)
            .password(secrets.password.as_deref().unwrap_or(""))
            .database(database)
            .application_name("Tablory")
            .ssl_mode(match profile.ssl_mode {
                SslMode::Disable => PgSslMode::Disable,
                SslMode::Prefer => PgSslMode::Prefer,
                SslMode::Require => PgSslMode::Require,
            })
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
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

    fn columns(row: &PgRow) -> Vec<ColumnMeta> {
        row.columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_owned(),
                type_name: c.type_info().name().to_owned(),
            })
            .collect()
    }

    /// Rows from the simple query protocol arrive as text, whatever the type.
    fn row(row: &PgRow, cols: &[ColumnMeta]) -> Vec<Value> {
        cols.iter()
            .enumerate()
            .map(|(i, c)| {
                let Ok(raw) = row.try_get_raw(i) else {
                    return Value::Null;
                };
                if sqlx::ValueRef::is_null(&raw) {
                    return Value::Null;
                }
                let Ok(s) = raw.as_str() else {
                    return raw.as_bytes().map(binary_cell).unwrap_or(Value::Null);
                };
                match c.type_name.as_str() {
                    "BOOL" => Value::Bool(s == "t"),
                    "INT2" | "INT4" | "OID" | "FLOAT4" | "FLOAT8" => number_or_string(s),
                    _ => Value::String(s.to_owned()),
                }
            })
            .collect()
    }

    async fn run_text(
        conn: &mut PgConnection,
        sql: &str,
        max_rows: usize,
    ) -> Result<Vec<StatementResult>> {
        let stream = sqlx::raw_sql(sql).fetch_many(&mut *conn);
        let mut out = collect_results(
            stream,
            max_rows,
            |q: &sqlx::postgres::PgQueryResult| q.rows_affected(),
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
impl Driver for PgDriver {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn current_database(&self) -> Result<String> {
        Ok(sqlx::query_scalar("SELECT current_database()::text")
            .fetch_one(&self.pool)
            .await?)
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT datname::text FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT nspname::text FROM pg_namespace
             WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema'
             ORDER BY nspname <> 'public', 1",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT c.relname::text, CASE WHEN c.relkind IN ('v','m') THEN 'view' ELSE 'table' END
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m','f') AND NOT c.relispartition
             ORDER BY 1",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, kind)| TableInfo { name, kind })
            .collect())
    }

    async fn table_structure(&self, t: &TableRef) -> Result<TableStructure> {
        const REL: &str = "to_regclass(format('%I.%I', $1::text, $2::text))";
        let columns: Vec<(String, String, bool, Option<String>, bool)> = sqlx::query_as(&format!(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull,
                    pg_get_expr(d.adbin, d.adrelid),
                    EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid
                            AND i.indisprimary AND a.attnum = ANY(i.indkey))
             FROM pg_attribute a
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             WHERE a.attrelid = {REL} AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum"
        ))
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;
        let indexes: Vec<(String, bool, bool, Vec<String>)> = sqlx::query_as(&format!(
            "SELECT ic.relname::text, i.indisunique, i.indisprimary,
                    ARRAY(SELECT COALESCE(a.attname::text, 'expr') FROM unnest(i.indkey) WITH ORDINALITY k(n, o)
                          LEFT JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.n
                          ORDER BY k.o)
             FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid
             WHERE i.indrelid = {REL} ORDER BY i.indisprimary DESC, 1"
        ))
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;
        let fks: Vec<(String, Vec<String>, String, Vec<String>)> = sqlx::query_as(&format!(
            "SELECT con.conname::text,
                    ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(n, o)
                          JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.n ORDER BY k.o),
                    con.confrelid::regclass::text,
                    ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(n, o)
                          JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.n ORDER BY k.o)
             FROM pg_constraint con WHERE con.contype = 'f' AND con.conrelid = {REL} ORDER BY 1"
        ))
        .bind(&t.schema)
        .bind(&t.name)
        .fetch_all(&self.pool)
        .await?;
        if columns.is_empty() {
            anyhow::bail!("table {}.{} not found", t.schema, t.name);
        }
        Ok(TableStructure {
            columns: columns
                .into_iter()
                .map(
                    |(name, data_type, nullable, default, primary_key)| ColumnInfo {
                        name,
                        data_type,
                        nullable,
                        default,
                        primary_key,
                    },
                )
                .collect(),
            indexes: indexes
                .into_iter()
                .map(|(name, unique, primary, columns)| IndexInfo {
                    name,
                    columns,
                    unique,
                    primary,
                })
                .collect(),
            foreign_keys: fks
                .into_iter()
                .map(|(name, columns, ref_table, ref_columns)| ForeignKeyInfo {
                    name,
                    columns,
                    ref_table,
                    ref_columns,
                })
                .collect(),
        })
    }

    async fn execute(&self, sql: &str, max_rows: usize, query_id: &str) -> Result<ExecuteResult> {
        let mut guard = self.editor.lock().await;
        if guard.is_none() {
            let mut conn = self.options.connect().await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut conn)
                .await?;
            *guard = Some((conn, pid));
        }
        let (conn, pid) = guard.as_mut().expect("editor connection was just set");
        self.running
            .lock()
            .unwrap()
            .insert(query_id.to_owned(), *pid);
        let start = Instant::now();
        let res = Self::run_text(conn, sql, max_rows).await;
        self.running.lock().unwrap().remove(query_id);
        if let Err(e) = &res {
            // A broken connection is replaced on the next run.
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
        let pid = self.running.lock().unwrap().get(query_id).copied();
        if let Some(pid) = pid {
            sqlx::query("SELECT pg_cancel_backend($1)")
                .bind(pid)
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
            let (sql, params) = build_change(Dialect::Postgres, table, change, &s, false);
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

/// An edit that matched zero or several rows means the data changed under us
/// (or the key is not unique); roll back rather than guess.
pub(crate) fn ensure_one_row(change: &RowChange, n: u64) -> Result<()> {
    if !matches!(change, RowChange::Insert { .. }) && n != 1 {
        anyhow::bail!("expected to change 1 row but matched {n}; nothing was saved");
    }
    Ok(())
}
