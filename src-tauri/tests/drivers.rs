//! Driver integration tests. SQLite always runs; the server tests need the dev
//! stack (`docker compose -f dev/docker-compose.yml up -d`) and
//! `TABLORY_TEST_DOCKER=1`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tablory_lib::connections::{ConnectionProfile, DbKind, Secrets, SshAuth, SshConfig, SslMode};
use tablory_lib::db::{self, ColValue, Driver, Filter, RowChange, RowsRequest, Sort, TableRef};
use tablory_lib::session::Sessions;

fn docker() -> bool {
    std::env::var("TABLORY_TEST_DOCKER").is_ok()
}

fn pw(p: &str) -> Secrets {
    Secrets {
        password: Some(p.into()),
        ..Default::default()
    }
}

fn pg() -> ConnectionProfile {
    ConnectionProfile {
        name: "pg".into(),
        kind: DbKind::Postgres,
        host: "127.0.0.1".into(),
        port: 54329,
        user: "postgres".into(),
        database: "shop".into(),
        ssl_mode: SslMode::Disable,
        ..Default::default()
    }
}

fn mysql() -> ConnectionProfile {
    ConnectionProfile {
        name: "mysql".into(),
        kind: DbKind::Mysql,
        host: "127.0.0.1".into(),
        port: 33069,
        user: "root".into(),
        database: "shop".into(),
        ssl_mode: SslMode::Disable,
        ..Default::default()
    }
}

async fn driver(p: &ConnectionProfile) -> Arc<dyn Driver> {
    db::connect(p, &pw("tablory"), (p.host.clone(), p.port), &p.database)
        .await
        .expect("connect")
}

fn table(schema: &str, name: &str) -> TableRef {
    TableRef {
        schema: schema.into(),
        name: name.into(),
    }
}

fn rows(t: TableRef) -> RowsRequest {
    RowsRequest {
        table: t,
        filters: vec![],
        raw_where: None,
        sort: None,
        limit: 100,
        offset: 0,
    }
}

fn cv(c: &str, v: Option<&str>) -> ColValue {
    ColValue {
        column: c.into(),
        value: v.map(Into::into),
    }
}

fn col(rs: &db::ResultSet, name: &str) -> usize {
    rs.columns.iter().position(|c| c.name == name).unwrap()
}

/// Browse, structure, edit and editor checks shared by every SQL engine.
async fn exercise(d: &dyn Driver, schema: &str) {
    let tables = d.list_tables(schema).await.unwrap();
    let kinds: Vec<(&str, &str)> = tables
        .iter()
        .map(|t| (t.name.as_str(), t.kind.as_str()))
        .collect();
    assert!(kinds.contains(&("customers", "table")), "{kinds:?}");
    assert!(kinds.contains(&("vip_customers", "view")), "{kinds:?}");

    let customers = table(schema, "customers");
    let s = d.table_structure(&customers).await.unwrap();
    assert_eq!(s.primary_key(), vec!["id"]);
    assert!(s.indexes.iter().any(|i| i.primary));
    let orders = d.table_structure(&table(schema, "orders")).await.unwrap();
    assert_eq!(orders.foreign_keys[0].columns, vec!["customer_id"]);

    // Filter + sort + paging.
    let mut req = rows(customers.clone());
    req.filters = vec![Filter {
        column: "name".into(),
        op: "contains".into(),
        value: Some("customer 1".into()),
    }];
    req.sort = Some(Sort {
        column: "id".into(),
        desc: true,
    });
    req.limit = 5;
    let rs = d.fetch_rows(&req).await.unwrap();
    assert_eq!(rs.rows.len(), 5);
    let id = col(&rs, "id");
    assert!(rs.rows[0][id].as_i64().unwrap() > rs.rows[1][id].as_i64().unwrap());
    assert!(d.count_rows(&req).await.unwrap() > 5);
    let avatar = &rs.rows[0][col(&rs, "avatar")];
    assert!(
        avatar.get("$bin").is_some() || avatar.as_str().is_some_and(|s| s.starts_with("\\x")),
        "{avatar}"
    );

    // Empty page still has headers.
    let mut empty = rows(customers.clone());
    empty.raw_where = Some("1 = 0".into());
    let rs = d.fetch_rows(&empty).await.unwrap();
    assert!(rs.rows.is_empty() && !rs.columns.is_empty());

    // Edits: update, insert, delete in one transaction.
    let changes = vec![
        RowChange::Update {
            key: vec![cv("id", Some("1"))],
            values: vec![cv("name", Some("Renamed 'one'")), cv("email", None)],
        },
        RowChange::Insert {
            values: vec![
                cv("name", Some("Inserted")),
                cv("email", Some("new@example.com")),
            ],
        },
    ];
    let preview = d.preview_changes(&customers, &changes).await.unwrap();
    assert!(preview.contains("Renamed ''one''"), "{preview}");
    assert_eq!(d.apply_changes(&customers, &changes).await.unwrap(), 2);
    let mut one = rows(customers.clone());
    one.raw_where = Some("email IS NULL OR email = 'new@example.com'".into());
    one.sort = Some(Sort {
        column: "id".into(),
        desc: false,
    });
    let rs = d.fetch_rows(&one).await.unwrap();
    let name = col(&rs, "name");
    assert_eq!(rs.rows[0][name], json!("Renamed 'one'"));
    let new_id = rs.rows[1][col(&rs, "id")].to_string();
    d.apply_changes(
        &customers,
        &[RowChange::Delete {
            key: vec![cv("id", Some(&new_id))],
        }],
    )
    .await
    .unwrap();

    // A key that matches nothing rolls back the whole batch.
    let bad = vec![
        RowChange::Update {
            key: vec![cv("id", Some("2"))],
            values: vec![cv("name", Some("should not stick"))],
        },
        RowChange::Delete {
            key: vec![cv("id", Some("999999"))],
        },
    ];
    assert!(d.apply_changes(&customers, &bad).await.is_err());
    let mut two = rows(customers.clone());
    two.raw_where = Some("id = 2".into());
    let rs = d.fetch_rows(&two).await.unwrap();
    assert_eq!(rs.rows[0][col(&rs, "name")], json!("Customer 2"));

    // Tables without a primary key are read-only.
    let err = d
        .apply_changes(
            &table(schema, "audit_log"),
            &[RowChange::Delete {
                key: vec![cv("message", Some("x"))],
            }],
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("primary key"));

    // Editor: several statements, truncation, empty result headers, errors.
    let r = d
        .execute(
            "SELECT 1 AS a; UPDATE audit_log SET message = message; SELECT name FROM customers",
            10,
            "q1",
        )
        .await
        .unwrap();
    assert!(r.statements.len() >= 2, "{r:?}");
    assert_eq!(
        r.statements[0].result.as_ref().unwrap().rows[0][0],
        json!(1)
    );
    let last = r.statements.last().unwrap().result.as_ref().unwrap();
    assert!(last.truncated && last.rows.len() == 10);
    let r = d
        .execute("SELECT * FROM customers WHERE 1 = 0", 10, "q2")
        .await
        .unwrap();
    assert!(!r.statements[0].result.as_ref().unwrap().columns.is_empty());
    assert!(d.execute("SELEC nope", 10, "q3").await.is_err());
    // The editor connection survives an error.
    assert!(d.execute("SELECT 2", 10, "q4").await.is_ok());
}

#[tokio::test]
async fn postgres() {
    if !docker() {
        return;
    }
    let d = driver(&pg()).await;
    assert_eq!(d.current_database().await.unwrap(), "shop");
    let schemas = d.list_schemas().await.unwrap();
    assert_eq!(schemas[0], "public");
    assert!(schemas.contains(&"reporting".to_string()));
    exercise(d.as_ref(), "public").await;

    // Type mapping from the text protocol.
    let mut req = rows(table("public", "customers"));
    req.raw_where = Some("id = 7".into());
    let rs = d.fetch_rows(&req).await.unwrap();
    let r = &rs.rows[0];
    assert_eq!(r[col(&rs, "vip")], json!(true));
    assert_eq!(r[col(&rs, "big")], json!("9007199254741000"));
    assert_eq!(r[col(&rs, "tags")], json!("{a,b}"));
    assert_eq!(r[col(&rs, "feeling")], json!("sad"));
    assert_eq!(r[col(&rs, "meta")], json!(r#"{"n": 7}"#));

    // Typed values travel as text and are cast back (same values, so reruns pass).
    d.apply_changes(
        &table("public", "customers"),
        &[RowChange::Update {
            key: vec![cv("id", Some("7"))],
            values: vec![
                cv("vip", Some("true")),
                cv("feeling", Some("sad")),
                cv("tags", Some("{a,b}")),
                cv("meta", Some(r#"{"n": 7}"#)),
                cv("avatar", Some("\\xdeadbeef")),
            ],
        }],
    )
    .await
    .unwrap();

    // Quoted identifiers.
    let weird = table("reporting", "Weird \"Name\"");
    let rs = d.fetch_rows(&rows(weird.clone())).await.unwrap();
    assert_eq!(rs.rows[0][col(&rs, "Mixed Case")], json!("it's quoted"));
    d.apply_changes(
        &weird,
        &[RowChange::Update {
            key: vec![cv("id", Some("1"))],
            values: vec![cv("Mixed Case", Some("it's quoted"))],
        }],
    )
    .await
    .unwrap();

    // Cancelling a long query.
    let d2 = d.clone();
    let run = tokio::spawn(async move { d2.execute("SELECT pg_sleep(30)", 10, "sleep").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    d.cancel("sleep").await.unwrap();
    let res = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert!(format!("{:#}", res.unwrap_err()).contains("cancel"));

    // BEGIN/ROLLBACK across runs share the editor connection.
    d.execute("BEGIN; DELETE FROM orders", 10, "t1")
        .await
        .unwrap();
    d.execute("ROLLBACK", 10, "t2").await.unwrap();
    assert_eq!(
        d.count_rows(&rows(table("public", "orders")))
            .await
            .unwrap(),
        20000
    );
    d.close().await;
}

#[tokio::test]
async fn mysql_and_mariadb_dialect() {
    if !docker() {
        return;
    }
    let d = driver(&mysql()).await;
    assert_eq!(d.current_database().await.unwrap(), "shop");
    assert!(d
        .list_schemas()
        .await
        .unwrap()
        .contains(&"shop".to_string()));
    exercise(d.as_ref(), "shop").await;

    let mut req = rows(table("shop", "customers"));
    req.raw_where = Some("id = 7".into());
    let rs = d.fetch_rows(&req).await.unwrap();
    let r = &rs.rows[0];
    assert_eq!(r[col(&rs, "vip")], json!(1));
    assert!(r[col(&rs, "big")].is_string());
    assert_eq!(r[col(&rs, "balance")], json!("10.50"));
    assert_eq!(r[col(&rs, "avatar")]["$bin"], json!(4));

    let d2 = d.clone();
    let run = tokio::spawn(async move { d2.execute("SELECT SLEEP(30)", 10, "sleep").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    d.cancel("sleep").await.unwrap();
    // KILL QUERY makes SLEEP return 1 early instead of failing.
    let res = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert!(res.is_ok() || format!("{:#}", res.unwrap_err()).contains("interrupted"));
    d.close().await;
}

fn mssql(database: &str) -> ConnectionProfile {
    ConnectionProfile {
        name: "mssql".into(),
        kind: DbKind::Mssql,
        host: "127.0.0.1".into(),
        port: 14339,
        user: "sa".into(),
        database: database.into(),
        ssl_mode: SslMode::Prefer,
        ..Default::default()
    }
}

const MSSQL_SEED: &str = "
IF OBJECT_ID('dbo.vip_customers') IS NOT NULL DROP VIEW dbo.vip_customers;
IF OBJECT_ID('dbo.orders') IS NOT NULL DROP TABLE dbo.orders;
IF OBJECT_ID('dbo.customers') IS NOT NULL DROP TABLE dbo.customers;
IF OBJECT_ID('dbo.audit_log') IS NOT NULL DROP TABLE dbo.audit_log;
CREATE TABLE dbo.customers (
  id int IDENTITY PRIMARY KEY,
  name nvarchar(100) NOT NULL,
  email nvarchar(200),
  vip bit NOT NULL DEFAULT 0,
  balance decimal(12, 2) DEFAULT 0,
  big bigint,
  uid uniqueidentifier DEFAULT NEWID(),
  avatar varbinary(max),
  created_at datetime2(3) DEFAULT SYSDATETIME()
);
CREATE TABLE dbo.orders (id int IDENTITY PRIMARY KEY, customer_id int NOT NULL REFERENCES dbo.customers(id), total float);
CREATE INDEX orders_customer_idx ON dbo.orders(customer_id);
CREATE TABLE dbo.audit_log (at datetime2, message nvarchar(max));
WITH seq AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM seq WHERE n < 1000)
INSERT INTO dbo.customers (name, email, vip, balance, big, avatar)
SELECT CONCAT('Customer ', n), CONCAT('c', n, '@example.com'), CASE WHEN n % 7 = 0 THEN 1 ELSE 0 END, n * 1.5,
       9007199254740993 + n, 0xDEADBEEF FROM seq OPTION (MAXRECURSION 1000);
INSERT INTO dbo.orders (customer_id, total) SELECT id, id * 0.25 FROM dbo.customers;
INSERT INTO dbo.audit_log VALUES (SYSDATETIME(), N'no primary key here');
GO
CREATE VIEW dbo.vip_customers AS SELECT * FROM dbo.customers WHERE vip = 1;
";

#[tokio::test]
async fn sql_server() {
    if !docker() {
        return;
    }
    let master = db::connect(
        &mssql(""),
        &pw("Tablory!2026"),
        ("127.0.0.1".into(), 14339),
        "",
    )
    .await
    .expect("connect");
    master
        .execute("IF DB_ID('shop') IS NULL CREATE DATABASE shop", 10, "db")
        .await
        .unwrap();
    master.close().await;

    let d = db::connect(
        &mssql("shop"),
        &pw("Tablory!2026"),
        ("127.0.0.1".into(), 14339),
        "shop",
    )
    .await
    .unwrap();
    d.execute(MSSQL_SEED, 10, "seed").await.unwrap();
    assert_eq!(d.current_database().await.unwrap(), "shop");
    assert_eq!(d.list_schemas().await.unwrap()[0], "dbo");
    assert!(d
        .list_databases()
        .await
        .unwrap()
        .contains(&"shop".to_string()));
    exercise(d.as_ref(), "dbo").await;

    let s = d.table_structure(&table("dbo", "customers")).await.unwrap();
    let name = s.columns.iter().find(|c| c.name == "name").unwrap();
    assert_eq!(name.data_type, "nvarchar(100)");
    assert_eq!(s.columns[0].default.as_deref(), Some("identity"));

    let mut req = rows(table("dbo", "customers"));
    req.raw_where = Some("id = 7".into());
    let rs = d.fetch_rows(&req).await.unwrap();
    let r = &rs.rows[0];
    assert_eq!(r[col(&rs, "vip")], json!(true));
    assert_eq!(r[col(&rs, "balance")], json!("10.50"));
    assert_eq!(r[col(&rs, "big")], json!("9007199254741000"));
    assert!(r[col(&rs, "created_at")].as_str().unwrap().len() >= 19);
    assert_eq!(r[col(&rs, "uid")].as_str().unwrap().len(), 36);

    // Typed values round-trip as text through CAST.
    d.apply_changes(
        &table("dbo", "customers"),
        &[RowChange::Update {
            key: vec![cv("id", Some("7"))],
            values: vec![
                cv("vip", Some("true")),
                cv("balance", Some("10.50")),
                cv("created_at", Some("2026-01-02 03:04:05.678")),
                cv("name", Some("Zoë ☕")),
            ],
        }],
    )
    .await
    .unwrap();
    let rs = d.fetch_rows(&req).await.unwrap();
    assert_eq!(rs.rows[0][col(&rs, "name")], json!("Zoë ☕"));
    assert_eq!(
        rs.rows[0][col(&rs, "created_at")],
        json!("2026-01-02 03:04:05.678")
    );

    // Data-changing batches report affected rows; GO splits batches.
    let r = d
        .execute("UPDATE dbo.orders SET total = total WHERE id <= 3\nGO\nSELECT COUNT(*) AS n FROM dbo.orders", 10, "b")
        .await
        .unwrap();
    assert_eq!(r.statements[0].rows_affected, 3);
    assert_eq!(
        r.statements[1].result.as_ref().unwrap().rows[0][0],
        json!(1000)
    );

    // Cancelling kills the editor session; the next run reconnects.
    let d2 = d.clone();
    let run =
        tokio::spawn(async move { d2.execute("WAITFOR DELAY '00:00:30'", 10, "sleep").await });
    tokio::time::sleep(Duration::from_millis(800)).await;
    d.cancel("sleep").await.unwrap();
    let res = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap();
    assert!(format!("{:#}", res.unwrap_err()).contains("cancelled"));
    assert!(d.execute("SELECT 1", 10, "after").await.is_ok());
    d.close().await;
}

#[tokio::test]
async fn ssh_tunnel_password_and_key() {
    if !docker() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("tablory-test-{}", std::process::id()));
    let sessions = Sessions::new(dir);
    let mut p = pg();
    p.host = "postgres".into(); // only resolvable from the bastion
    p.port = 5432;
    p.ssh = SshConfig {
        enabled: true,
        host: "127.0.0.1".into(),
        port: 22229,
        user: "tablory".into(),
        auth: SshAuth::Password,
        key_path: String::new(),
    };
    let secrets = Secrets {
        password: Some("tablory".into()),
        ssh_password: Some("tablory".into()),
        ssh_passphrase: None,
    };
    assert_eq!(sessions.test(&p, &secrets).await.unwrap(), "shop");

    p.ssh.auth = SshAuth::Key;
    p.ssh.key_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../dev/ssh/id_ed25519").into();
    let info = sessions.connect(p.clone(), secrets.clone()).await.unwrap();
    let s = sessions.get(&info.id).await.unwrap();
    assert_eq!(
        s.driver()
            .await
            .unwrap()
            .count_rows(&rows(table("public", "orders")))
            .await
            .unwrap(),
        20000
    );
    assert_eq!(
        sessions
            .switch_database(&info.id, "postgres")
            .await
            .unwrap(),
        "postgres"
    );
    sessions.disconnect(&info.id).await;

    let mut wrong = secrets.clone();
    wrong.ssh_password = Some("nope".into());
    p.ssh.auth = SshAuth::Password;
    let err = sessions.test(&p, &wrong).await.unwrap_err();
    assert!(err.to_string().contains("authentication failed"), "{err:#}");
}

#[tokio::test]
async fn sqlite() {
    let file = std::env::temp_dir().join(format!("tablory-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&file);
    std::fs::File::create(&file).unwrap();
    let p = ConnectionProfile {
        kind: DbKind::Sqlite,
        file: file.display().to_string(),
        ..Default::default()
    };
    let d = db::connect(&p, &Secrets::default(), (String::new(), 0), "")
        .await
        .unwrap();
    d.execute(
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT, vip INTEGER DEFAULT 0, balance REAL, avatar BLOB);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), total REAL);
         CREATE TABLE audit_log (at TEXT, message TEXT);
         CREATE VIEW vip_customers AS SELECT * FROM customers WHERE vip;
         WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 300)
         INSERT INTO customers (name, email, vip, balance, avatar)
           SELECT 'Customer ' || n, 'c' || n || '@example.com', n % 7 = 0, n * 1.5, x'deadbeef' FROM seq;
         INSERT INTO orders (customer_id, total) SELECT id, id * 0.25 FROM customers;
         INSERT INTO audit_log VALUES (datetime(), 'no primary key here');",
        10,
        "setup",
    )
    .await
    .unwrap();
    assert_eq!(d.list_schemas().await.unwrap(), vec!["main"]);
    exercise(d.as_ref(), "main").await;
    let rs = d
        .query("SELECT balance, avatar FROM customers WHERE id = 3", 1)
        .await
        .unwrap();
    assert_eq!(rs.rows[0][0], json!(4.5));
    assert_eq!(rs.rows[0][1]["hex"], Value::String("deadbeef".into()));
    d.close().await;
    let _ = std::fs::remove_file(&file);
}

fn ssh_bastion() -> SshConfig {
    SshConfig {
        enabled: true,
        host: "127.0.0.1".into(),
        port: 22229,
        user: "tablory".into(),
        auth: SshAuth::Password,
        key_path: String::new(),
    }
}

#[tokio::test]
async fn redis() {
    if !docker() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("tablory-redis-{}", std::process::id()));
    let sessions = Sessions::new(dir);
    let p = ConnectionProfile {
        kind: DbKind::Redis,
        host: "127.0.0.1".into(),
        port: 63799,
        database: "2".into(),
        ssl_mode: SslMode::Disable,
        ..Default::default()
    };
    let info = sessions.connect(p.clone(), pw("tablory")).await.unwrap();
    assert_eq!(info.database, "2");
    let r = sessions
        .get(&info.id)
        .await
        .unwrap()
        .backend()
        .await
        .redis()
        .unwrap();
    let cmd = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    r.command(&cmd(&["FLUSHDB"])).await.unwrap();
    r.command(&cmd(&["SET", "user:1:name", "Zoë"]))
        .await
        .unwrap();
    r.command(&cmd(&["EXPIRE", "user:1:name", "300"]))
        .await
        .unwrap();
    r.command(&cmd(&["HSET", "user:1", "name", "Ann", "age", "30"]))
        .await
        .unwrap();
    r.command(&cmd(&["RPUSH", "queue", "a", "b", "c"]))
        .await
        .unwrap();
    r.command(&cmd(&["SADD", "tags", "x", "y"])).await.unwrap();
    r.command(&cmd(&["ZADD", "rank", "1.5", "p1", "3", "p2"]))
        .await
        .unwrap();
    r.command(&cmd(&["XADD", "events", "*", "kind", "login"]))
        .await
        .unwrap();
    for i in 0..300 {
        r.command(&cmd(&["SET", &format!("bulk:{i}"), "v"]))
            .await
            .unwrap();
    }

    // SCAN pages until the cursor returns to 0.
    let mut cursor = "0".to_string();
    let mut keys = Vec::new();
    loop {
        let page = r.scan(&cursor, "bulk:*", 100).await.unwrap();
        keys.extend(page.keys);
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(keys.len(), 300);
    assert!(keys.iter().all(|k| k.kind == "string"));

    let v = r.get("user:1:name").await.unwrap();
    assert_eq!((v.kind.as_str(), v.value.clone()), ("string", json!("Zoë")));
    assert!(v.ttl > 0 && v.ttl <= 300);
    let v = r.get("user:1").await.unwrap();
    assert_eq!(v.length, 2);
    assert!(v.value.as_array().unwrap().contains(&json!(["age", "30"])));
    assert_eq!(r.get("queue").await.unwrap().value, json!(["a", "b", "c"]));
    assert_eq!(r.get("tags").await.unwrap().value, json!(["x", "y"]));
    assert_eq!(
        r.get("rank").await.unwrap().value,
        json!([["p1", "1.5"], ["p2", "3"]])
    );
    let ev = r.get("events").await.unwrap();
    assert_eq!(ev.value[0][1], json!({"kind": "login"}));
    assert!(r.get("missing").await.is_err());
    assert!(r.command(&cmd(&["SELECT", "1"])).await.is_err());
    assert_eq!(
        r.command(&tablory_lib::kv::split_args("GET \"user:1:name\"").unwrap())
            .await
            .unwrap(),
        json!("Zoë")
    );

    // Switching database keeps the session.
    assert_eq!(sessions.switch_database(&info.id, "3").await.unwrap(), "3");
    let r3 = sessions
        .get(&info.id)
        .await
        .unwrap()
        .backend()
        .await
        .redis()
        .unwrap();
    assert_eq!(
        r3.command(&cmd(&["EXISTS", "queue"])).await.unwrap(),
        json!(0)
    );
    sessions.disconnect(&info.id).await;

    // Through the SSH bastion, by service name.
    let mut tunneled = p.clone();
    tunneled.host = "redis".into();
    tunneled.port = 6379;
    tunneled.ssh = ssh_bastion();
    let secrets = Secrets {
        password: Some("tablory".into()),
        ssh_password: Some("tablory".into()),
        ssh_passphrase: None,
    };
    assert_eq!(sessions.test(&tunneled, &secrets).await.unwrap(), "2");
    assert!(sessions.test(&p, &pw("wrong")).await.is_err());
}

#[tokio::test]
async fn mongodb() {
    if !docker() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("tablory-mongo-{}", std::process::id()));
    let sessions = Sessions::new(dir);
    let p = ConnectionProfile {
        kind: DbKind::Mongodb,
        host: "127.0.0.1".into(),
        port: 27019,
        user: "root".into(),
        database: "shop".into(),
        ssl_mode: SslMode::Disable,
        ..Default::default()
    };
    let info = sessions.connect(p.clone(), pw("tablory")).await.unwrap();
    assert_eq!(info.database, "shop");
    let m = sessions
        .get(&info.id)
        .await
        .unwrap()
        .backend()
        .await
        .mongo()
        .unwrap();
    m.command("shop", "{dropDatabase: 1}").await.unwrap();
    for i in 1..=250 {
        m.insert(
            "shop",
            "customers",
            &format!("{{name: 'Customer {i}', n: {i}, score: 1.0, vip: {}, tags: ['a'], at: {{$date: '2026-01-01T00:00:00Z'}}}}", i % 7 == 0),
        )
        .await
        .unwrap();
    }
    m.command(
        "shop",
        "{create: 'vip', viewOn: 'customers', pipeline: [{$match: {vip: true}}]}",
    )
    .await
    .unwrap();

    assert!(m.databases().await.unwrap().contains(&"shop".to_string()));
    let colls = m.collections("shop").await.unwrap();
    let kinds: Vec<(&str, &str)> = colls
        .iter()
        .map(|c| (c.name.as_str(), c.kind.as_str()))
        .collect();
    assert_eq!(kinds, vec![("customers", "table"), ("vip", "view")]);

    assert_eq!(m.count("shop", "customers", "").await.unwrap(), 250);
    assert_eq!(
        m.count("shop", "customers", "{vip: true}").await.unwrap(),
        35
    );
    let page = m
        .find("shop", "customers", "{n: {$gt: 10}}", "{n: -1}", 5, 10)
        .await
        .unwrap();
    assert_eq!(page.docs.len(), 10);
    assert_eq!(page.docs[0]["n"], json!(245));
    assert!(page.docs[0]["_id"]["$oid"].is_string());
    assert!(page.docs[0]["at"]["$date"].is_string());
    // The edit text keeps 1.0 a double.
    assert!(page.texts[0].contains("\"score\": 1.0"));

    // Replace by _id keeps types and rejects _id changes.
    let id = page.docs[0]["_id"].to_string();
    let edited = page.texts[0].replace("\"Customer 245\"", "\"Renamed\"");
    m.replace("shop", "customers", &id, &edited).await.unwrap();
    let again = m
        .find("shop", "customers", &format!("{{_id: {id}}}"), "", 0, 1)
        .await
        .unwrap();
    assert_eq!(again.docs[0]["name"], json!("Renamed"));
    assert!(again.texts[0].contains("\"score\": 1.0"));
    let bad = edited.replace(
        &page.docs[0]["_id"]["$oid"].as_str().unwrap()[..6],
        "000000",
    );
    assert!(m.replace("shop", "customers", &id, &bad).await.is_err());

    let new_id = m
        .insert("shop", "customers", "{name: 'temp'}")
        .await
        .unwrap();
    assert_eq!(
        m.delete("shop", "customers", &[new_id.to_string(), id])
            .await
            .unwrap(),
        2
    );
    assert!(m
        .find("shop", "customers", "{name: ", "", 0, 1)
        .await
        .is_err());
    sessions.disconnect(&info.id).await;

    let mut tunneled = p.clone();
    tunneled.host = "mongo".into();
    tunneled.port = 27017;
    tunneled.ssh = ssh_bastion();
    let secrets = Secrets {
        password: Some("tablory".into()),
        ssh_password: Some("tablory".into()),
        ssh_passphrase: None,
    };
    assert_eq!(sessions.test(&tunneled, &secrets).await.unwrap(), "shop");

    let mut by_uri = p.clone();
    by_uri.uri =
        "mongodb://root:tablory@127.0.0.1:27019/shop?authSource=admin&directConnection=true".into();
    by_uri.database = String::new();
    assert_eq!(
        sessions.test(&by_uri, &Secrets::default()).await.unwrap(),
        "shop"
    );
}
