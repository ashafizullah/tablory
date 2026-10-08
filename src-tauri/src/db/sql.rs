//! SQL text generation shared by the SQL drivers: identifier quoting, the
//! table browser's SELECT, and the INSERT/UPDATE/DELETE behind inline edits.

use anyhow::{bail, Result};

use super::{Dialect, RowChange, RowsRequest, TableRef, TableStructure};

pub fn quote_ident(d: Dialect, s: &str) -> String {
    match d {
        Dialect::Mysql => format!("`{}`", s.replace('`', "``")),
        Dialect::Postgres | Dialect::Sqlite => format!("\"{}\"", s.replace('"', "\"\"")),
        Dialect::Mssql => format!("[{}]", s.replace(']', "]]")),
    }
}

pub fn quote_literal(d: Dialect, s: &str) -> String {
    match d {
        // MySQL treats backslash as an escape unless NO_BACKSLASH_ESCAPES is set.
        Dialect::Mysql => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''")),
        Dialect::Postgres | Dialect::Sqlite => format!("'{}'", s.replace('\'', "''")),
        // N'' keeps non-ASCII text intact in nvarchar columns.
        Dialect::Mssql => format!("N'{}'", s.replace('\'', "''")),
    }
}

pub fn qualified(d: Dialect, t: &TableRef) -> String {
    if t.schema.is_empty() {
        quote_ident(d, &t.name)
    } else {
        format!("{}.{}", quote_ident(d, &t.schema), quote_ident(d, &t.name))
    }
}

fn literal_or_null(d: Dialect, v: &Option<String>) -> String {
    v.as_deref()
        .map_or_else(|| "NULL".to_owned(), |s| quote_literal(d, s))
}

/// Column as text, for LIKE on non-text columns.
fn as_text(d: Dialect, col: &str) -> String {
    match d {
        Dialect::Postgres => format!("{col}::text"),
        Dialect::Mysql => format!("CAST({col} AS CHAR)"),
        Dialect::Sqlite => col.to_owned(),
        Dialect::Mssql => format!("CAST({col} AS NVARCHAR(MAX))"),
    }
}

fn where_clause(d: Dialect, req: &RowsRequest) -> String {
    let mut parts = Vec::new();
    for f in &req.filters {
        let col = quote_ident(d, &f.column);
        let lit = || literal_or_null(d, &f.value);
        let like = |pattern: String| {
            let op = if d == Dialect::Postgres {
                "ILIKE"
            } else {
                "LIKE"
            };
            format!("{} {op} {}", as_text(d, &col), quote_literal(d, &pattern))
        };
        let v = f.value.clone().unwrap_or_default();
        parts.push(match f.op.as_str() {
            "=" | "!=" | "<" | ">" | "<=" | ">=" => {
                let op = if f.op == "!=" { "<>" } else { f.op.as_str() };
                format!("{col} {op} {}", lit())
            }
            "contains" => like(format!("%{v}%")),
            "not_contains" => format!("NOT ({})", like(format!("%{v}%"))),
            "starts_with" => like(format!("{v}%")),
            "ends_with" => like(format!("%{v}")),
            "like" => like(v),
            "is_null" => format!("{col} IS NULL"),
            "is_not_null" => format!("{col} IS NOT NULL"),
            _ => continue,
        });
    }
    if let Some(raw) = req.raw_where.as_deref().map(str::trim) {
        if !raw.is_empty() {
            parts.push(format!("({raw})"));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    }
}

pub fn build_select(d: Dialect, req: &RowsRequest) -> String {
    let mut sql = format!(
        "SELECT * FROM {}{}",
        qualified(d, &req.table),
        where_clause(d, req)
    );
    if let Some(s) = &req.sort {
        sql += &format!(
            " ORDER BY {} {}",
            quote_ident(d, &s.column),
            if s.desc { "DESC" } else { "ASC" }
        );
    } else if d == Dialect::Mssql {
        // OFFSET/FETCH needs an ORDER BY; this one keeps the server's order.
        sql += " ORDER BY (SELECT NULL)";
    }
    if d == Dialect::Mssql {
        sql += &format!(
            " OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
            req.offset, req.limit
        );
    } else {
        sql += &format!(" LIMIT {} OFFSET {}", req.limit, req.offset);
    }
    sql
}

pub fn build_count(d: Dialect, req: &RowsRequest) -> String {
    // COUNT(*) is a 32-bit int on SQL Server.
    let count = if d == Dialect::Mssql {
        "COUNT_BIG(*)"
    } else {
        "COUNT(*)"
    };
    format!(
        "SELECT {count} FROM {}{}",
        qualified(d, &req.table),
        where_clause(d, req)
    )
}

/// Builds one change. With `inline` the values are written as literals (for the
/// preview); otherwise they are placeholders and returned as parameters.
pub fn build_change(
    d: Dialect,
    table: &TableRef,
    change: &RowChange,
    s: &TableStructure,
    inline: bool,
) -> (String, Vec<Option<String>>) {
    let mut params = Vec::new();
    let mut value = |col: &str, v: &Option<String>| -> String {
        if inline {
            return literal_or_null(d, v);
        }
        params.push(v.clone());
        match d {
            // Values travel as text; Postgres and SQL Server need an explicit
            // cast to the column type (MySQL and SQLite convert implicitly).
            Dialect::Postgres => match s.column_type(col) {
                Some(t) => format!("CAST(${} AS {t})", params.len()),
                None => format!("${}", params.len()),
            },
            Dialect::Mssql => match s.column_type(col) {
                Some(t) => format!("CAST(@P{} AS {t})", params.len()),
                None => format!("@P{}", params.len()),
            },
            Dialect::Mysql | Dialect::Sqlite => "?".to_owned(),
        }
    };
    let t = qualified(d, table);
    let sql = match change {
        RowChange::Update { key, values } => {
            let set: Vec<String> = values
                .iter()
                .map(|cv| {
                    format!(
                        "{} = {}",
                        quote_ident(d, &cv.column),
                        value(&cv.column, &cv.value)
                    )
                })
                .collect();
            let cond = key_condition(d, key, &mut value);
            format!("UPDATE {t} SET {} WHERE {cond}", set.join(", "))
        }
        RowChange::Insert { values } if values.is_empty() => match d {
            Dialect::Mysql => format!("INSERT INTO {t} () VALUES ()"),
            _ => format!("INSERT INTO {t} DEFAULT VALUES"),
        },
        RowChange::Insert { values } => {
            let cols: Vec<String> = values.iter().map(|cv| quote_ident(d, &cv.column)).collect();
            let vals: Vec<String> = values
                .iter()
                .map(|cv| value(&cv.column, &cv.value))
                .collect();
            format!(
                "INSERT INTO {t} ({}) VALUES ({})",
                cols.join(", "),
                vals.join(", ")
            )
        }
        RowChange::Delete { key } => {
            let cond = key_condition(d, key, &mut value);
            format!("DELETE FROM {t} WHERE {cond}")
        }
    };
    (sql, params)
}

fn key_condition(
    d: Dialect,
    key: &[super::ColValue],
    value: &mut impl FnMut(&str, &Option<String>) -> String,
) -> String {
    key.iter()
        .map(|cv| match &cv.value {
            None => format!("{} IS NULL", quote_ident(d, &cv.column)),
            v => format!("{} = {}", quote_ident(d, &cv.column), value(&cv.column, v)),
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Edits address rows by primary key; refuse anything else.
pub fn check_change(s: &TableStructure, change: &RowChange) -> Result<()> {
    let key = match change {
        RowChange::Update { key, .. } | RowChange::Delete { key } => key,
        RowChange::Insert { .. } => return Ok(()),
    };
    let pk = s.primary_key();
    if pk.is_empty() {
        bail!("this table has no primary key, so rows cannot be edited safely");
    }
    let mut cols: Vec<&str> = key.iter().map(|c| c.column.as_str()).collect();
    cols.sort_unstable();
    let mut pk_sorted = pk.clone();
    pk_sorted.sort_unstable();
    if cols != pk_sorted {
        bail!("row key must be the primary key ({})", pk.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ColValue, ColumnInfo, Filter, Sort};

    fn structure() -> TableStructure {
        let col = |name: &str, t: &str, pk: bool| ColumnInfo {
            name: name.into(),
            data_type: t.into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
        };
        TableStructure {
            columns: vec![col("id", "integer", true), col("name", "text", false)],
            indexes: vec![],
            foreign_keys: vec![],
        }
    }

    fn table() -> TableRef {
        TableRef {
            schema: "public".into(),
            name: "users".into(),
        }
    }

    fn cv(c: &str, v: Option<&str>) -> ColValue {
        ColValue {
            column: c.into(),
            value: v.map(Into::into),
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_ident(Dialect::Postgres, r#"a"b"#), r#""a""b""#);
        assert_eq!(quote_ident(Dialect::Mysql, "a`b"), "`a``b`");
        assert_eq!(quote_literal(Dialect::Sqlite, "it's"), "'it''s'");
        assert_eq!(quote_literal(Dialect::Mysql, r"a\'"), r"'a\\'''");
    }

    #[test]
    fn select_with_filters() {
        let req = RowsRequest {
            table: table(),
            filters: vec![
                Filter {
                    column: "name".into(),
                    op: "contains".into(),
                    value: Some("o'k".into()),
                },
                Filter {
                    column: "id".into(),
                    op: ">".into(),
                    value: Some("5".into()),
                },
            ],
            raw_where: Some(" ".into()),
            sort: Some(Sort {
                column: "id".into(),
                desc: true,
            }),
            limit: 100,
            offset: 200,
        };
        assert_eq!(
            build_select(Dialect::Postgres, &req),
            r#"SELECT * FROM "public"."users" WHERE "name"::text ILIKE '%o''k%' AND "id" > '5' ORDER BY "id" DESC LIMIT 100 OFFSET 200"#
        );
    }

    #[test]
    fn update_uses_casts_on_postgres() {
        let c = RowChange::Update {
            key: vec![cv("id", Some("7"))],
            values: vec![cv("name", None)],
        };
        let (sql, params) = build_change(Dialect::Postgres, &table(), &c, &structure(), false);
        assert_eq!(
            sql,
            r#"UPDATE "public"."users" SET "name" = CAST($1 AS text) WHERE "id" = CAST($2 AS integer)"#
        );
        assert_eq!(params, vec![None, Some("7".into())]);
        let (preview, _) = build_change(Dialect::Mysql, &table(), &c, &structure(), true);
        assert_eq!(
            preview,
            "UPDATE `public`.`users` SET `name` = NULL WHERE `id` = '7'"
        );
    }

    #[test]
    fn mssql_paging_and_params() {
        let req = RowsRequest {
            table: TableRef {
                schema: "dbo".into(),
                name: "a]b".into(),
            },
            filters: vec![],
            raw_where: None,
            sort: None,
            limit: 10,
            offset: 20,
        };
        assert_eq!(
            build_select(Dialect::Mssql, &req),
            "SELECT * FROM [dbo].[a]]b] ORDER BY (SELECT NULL) OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY"
        );
        let c = RowChange::Update {
            key: vec![cv("id", Some("7"))],
            values: vec![cv("name", Some("é"))],
        };
        let (sql, _) = build_change(Dialect::Mssql, &table(), &c, &structure(), false);
        assert_eq!(
            sql,
            "UPDATE [public].[users] SET [name] = CAST(@P1 AS text) WHERE [id] = CAST(@P2 AS integer)"
        );
        let (preview, _) = build_change(Dialect::Mssql, &table(), &c, &structure(), true);
        assert!(preview.contains("N'é'"));
    }

    #[test]
    fn insert_and_delete() {
        let ins = RowChange::Insert { values: vec![] };
        let (sql, _) = build_change(Dialect::Sqlite, &table(), &ins, &structure(), false);
        assert_eq!(sql, r#"INSERT INTO "public"."users" DEFAULT VALUES"#);
        let del = RowChange::Delete {
            key: vec![cv("id", Some("1"))],
        };
        let (sql, params) = build_change(Dialect::Mysql, &table(), &del, &structure(), false);
        assert_eq!(sql, "DELETE FROM `public`.`users` WHERE `id` = ?");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn edits_require_primary_key() {
        let c = RowChange::Delete {
            key: vec![cv("name", Some("x"))],
        };
        assert!(check_change(&structure(), &c).is_err());
        let ok = RowChange::Delete {
            key: vec![cv("id", Some("1"))],
        };
        assert!(check_change(&structure(), &ok).is_ok());
    }
}
