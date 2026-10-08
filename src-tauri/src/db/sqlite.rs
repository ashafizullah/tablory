use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqlitePool, SqlitePoolOptions, SqliteRow,
};
use sqlx::{Column, ConnectOptions, Connection, Executor, Row, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use super::postgres::ensure_one_row;
use super::sql::{build_change, check_change, quote_ident};
use super::*;

pub struct SqliteDriver {
    pool: SqlitePool,
    options: SqliteConnectOptions,
    file: String,
    editor: Mutex<Option<SqliteConnection>>,
}

impl SqliteDriver {
    pub async fn connect(profile: &ConnectionProfile) -> Result<Self> {
        let file = profile.file.trim();
        if file.is_empty() {
            bail!("choose a SQLite database file");
        }
        let options = SqliteConnectOptions::new()
            .filename(file)
            .create_if_missing(false)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .with_context(|| format!("cannot open {file}"))?;
        Ok(Self {
            pool,
            options,
            file: file.to_owned(),
            editor: Mutex::new(None),
        })
    }

    fn columns(row: &SqliteRow) -> Vec<ColumnMeta> {
        row.columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_owned(),
                type_name: c.type_info().name().to_owned(),
            })
            .collect()
    }

    /// SQLite values carry their own storage class, whatever the column says.
    fn row(row: &SqliteRow, cols: &[ColumnMeta]) -> Vec<Value> {
        (0..cols.len())
            .map(|i| {
                let Ok(raw) = row.try_get_raw(i) else {
                    return Value::Null;
                };
                if raw.is_null() {
                    return Value::Null;
                }
                let storage = raw.type_info().name().to_owned();
                match storage.as_str() {
                    "INTEGER" => row
                        .try_get_unchecked::<i64, _>(i)
                        .map(|n| {
                            if n.abs() <= 1 << 53 {
                                Value::from(n)
                            } else {
                                Value::String(n.to_string())
                            }
                        })
                        .unwrap_or(Value::Null),
                    "REAL" => row
                        .try_get_unchecked::<f64, _>(i)
                        .map(|f| number_or_string(&f.to_string()))
                        .unwrap_or(Value::Null),
                    "BLOB" => row
                        .try_get_unchecked::<Vec<u8>, _>(i)
                        .map(|b| binary_cell(&b))
                        .unwrap_or(Value::Null),
                    _ => row
                        .try_get_unchecked::<String, _>(i)
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                }
            })
            .collect()
    }

    async fn run(
        conn: &mut SqliteConnection,
        sql: &str,
        max_rows: usize,
    ) -> Result<Vec<StatementResult>> {
        let stream = sqlx::raw_sql(sql).fetch_many(&mut *conn);
        let mut out = collect_results(
            stream,
            max_rows,
            |q: &sqlx::sqlite::SqliteQueryResult| q.rows_affected(),
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
impl Driver for SqliteDriver {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn current_database(&self) -> Result<String> {
        Ok(std::path::Path::new(&self.file)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.file.clone()))
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![self.current_database().await?])
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_database_list WHERE name <> 'temp'")
                .fetch_all(&self.pool)
                .await?;
        Ok(names)
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let sql = format!(
            "SELECT name, type FROM {}.sqlite_master
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
            quote_ident(Dialect::Sqlite, schema)
        );
        let rows: Vec<(String, String)> = sqlx::query_as(&sql).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(name, kind)| TableInfo { name, kind })
            .collect())
    }

    async fn table_structure(&self, t: &TableRef) -> Result<TableStructure> {
        let cols: Vec<(String, String, i64, Option<String>, i64)> = sqlx::query_as(
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1, ?2) ORDER BY cid",
        )
        .bind(&t.name)
        .bind(&t.schema)
        .fetch_all(&self.pool)
        .await?;
        if cols.is_empty() {
            bail!("table {} not found", t.name);
        }
        let index_list: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT name, \"unique\", origin FROM pragma_index_list(?1, ?2) ORDER BY name",
        )
        .bind(&t.name)
        .bind(&t.schema)
        .fetch_all(&self.pool)
        .await?;
        let mut indexes = Vec::new();
        for (name, unique, origin) in index_list {
            let columns: Vec<String> = sqlx::query_scalar(
                "SELECT COALESCE(name, 'expr') FROM pragma_index_info(?1, ?2) ORDER BY seqno",
            )
            .bind(&name)
            .bind(&t.schema)
            .fetch_all(&self.pool)
            .await?;
            indexes.push(IndexInfo {
                name,
                columns,
                unique: unique != 0,
                primary: origin == "pk",
            });
        }
        // An INTEGER PRIMARY KEY is the rowid and has no index of its own.
        let mut pk: Vec<(i64, String)> = cols
            .iter()
            .filter(|c| c.4 > 0)
            .map(|c| (c.4, c.0.clone()))
            .collect();
        if !pk.is_empty() && !indexes.iter().any(|i| i.primary) {
            pk.sort();
            indexes.insert(
                0,
                IndexInfo {
                    name: "PRIMARY".into(),
                    columns: pk.into_iter().map(|(_, n)| n).collect(),
                    unique: true,
                    primary: true,
                },
            );
        }
        let fk_rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq",
        )
        .bind(&t.name)
        .bind(&t.schema)
        .fetch_all(&self.pool)
        .await?;
        let mut foreign_keys: Vec<(i64, ForeignKeyInfo)> = Vec::new();
        for (id, table, from, to) in fk_rows {
            let to = to.unwrap_or_default();
            match foreign_keys.last_mut() {
                Some((last, fk)) if *last == id => {
                    fk.columns.push(from);
                    fk.ref_columns.push(to);
                }
                _ => foreign_keys.push((
                    id,
                    ForeignKeyInfo {
                        name: format!("fk_{id}"),
                        columns: vec![from],
                        ref_table: table,
                        ref_columns: vec![to],
                    },
                )),
            }
        }
        Ok(TableStructure {
            columns: cols
                .into_iter()
                .map(|(name, data_type, notnull, default, pk)| ColumnInfo {
                    name,
                    data_type,
                    nullable: notnull == 0 && pk == 0,
                    default,
                    primary_key: pk > 0,
                })
                .collect(),
            indexes,
            foreign_keys: foreign_keys.into_iter().map(|(_, fk)| fk).collect(),
        })
    }

    async fn execute(&self, sql: &str, max_rows: usize, _query_id: &str) -> Result<ExecuteResult> {
        let mut guard = self.editor.lock().await;
        if guard.is_none() {
            *guard = Some(self.options.connect().await?);
        }
        let conn = guard.as_mut().expect("editor connection was just set");
        let start = Instant::now();
        let statements = Self::run(conn, sql, max_rows).await?;
        Ok(ExecuteResult {
            statements,
            duration_ms: elapsed_ms(start),
        })
    }

    async fn cancel(&self, _query_id: &str) -> Result<()> {
        bail!("SQLite queries cannot be cancelled")
    }

    async fn query(&self, sql: &str, max_rows: usize) -> Result<ResultSet> {
        let mut conn = self.pool.acquire().await?;
        let out = Self::run(&mut conn, sql, max_rows).await?;
        Ok(out.into_iter().find_map(|s| s.result).unwrap_or_default())
    }

    async fn apply_changes(&self, table: &TableRef, changes: &[RowChange]) -> Result<u64> {
        let s = self.table_structure(table).await?;
        let mut tx = self.pool.begin().await?;
        let mut total = 0;
        for change in changes {
            check_change(&s, change)?;
            let (sql, params) = build_change(Dialect::Sqlite, table, change, &s, false);
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
        if let Some(conn) = self.editor.lock().await.take() {
            let _ = conn.close().await;
        }
        self.pool.close().await;
    }
}
