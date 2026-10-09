//! Tauri commands. Thin wrappers: errors become strings for the frontend.

use std::collections::HashMap;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::connections::{self, ConnectionProfile, Secrets};
use crate::db::{
    ExecuteResult, ResultSet, RoutineInfo, RowChange, RowsRequest, TableInfo, TableRef,
    TableStructure,
};
use crate::docdb::FindResult;
use crate::kv::{self, KeyValue, ScanPage};
use crate::navicat;
use crate::session::{Backend, SessionInfo};
use crate::AppState;

type CmdResult<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub fn list_connections(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionProfile>> {
    state.store.list().map_err(err)
}

#[tauri::command]
pub fn save_connection(
    state: State<'_, AppState>,
    profile: ConnectionProfile,
    secrets: Secrets,
) -> CmdResult<ConnectionProfile> {
    let saved = state.store.save(profile).map_err(err)?;
    connections::save_secrets(&saved.id, secrets).map_err(err)?;
    Ok(saved)
}

#[tauri::command]
pub fn list_groups(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    state.store.groups().map_err(err)
}

#[tauri::command]
pub fn list_group_colors(state: State<'_, AppState>) -> CmdResult<HashMap<String, String>> {
    state.store.group_colors().map_err(err)
}

#[tauri::command]
pub fn set_group_color(state: State<'_, AppState>, name: String, color: String) -> CmdResult<()> {
    state.store.set_group_color(&name, &color).map_err(err)
}

#[tauri::command]
pub fn create_group(state: State<'_, AppState>, name: String) -> CmdResult<String> {
    state.store.create_group(&name).map_err(err)
}

#[tauri::command]
pub fn rename_group(state: State<'_, AppState>, from: String, to: String) -> CmdResult<()> {
    state.store.rename_group(&from, &to).map_err(err)
}

#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.store.delete_group(&name).map_err(err)
}

#[tauri::command]
pub fn move_connection(state: State<'_, AppState>, id: String, group: String) -> CmdResult<()> {
    state.store.move_connection(&id, &group).map_err(err)
}

#[tauri::command]
pub fn delete_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store.delete(&id).map_err(err)?;
    connections::delete_secrets(&id);
    Ok(())
}

/// Asks for a Navicat `.ncx` export and saves its connections. Connections
/// whose name already exists are skipped, so importing twice is harmless.
/// `None` when the dialog is cancelled.
#[tauri::command]
pub async fn import_navicat(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<navicat::ImportReport>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Navicat connections", &["ncx"])
        .pick_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let xml = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let (items, mut skipped) = navicat::parse(&xml).map_err(err)?;
    let existing = state.store.list().map_err(err)?;
    let mut imported = 0;
    for item in items {
        let name = &item.profile.name;
        if existing.iter().any(|c| &c.name == name) {
            skipped.push(format!(
                "{name}: a connection with this name already exists"
            ));
            continue;
        }
        let saved = state.store.save(item.profile).map_err(err)?;
        connections::save_secrets(&saved.id, item.secrets).map_err(err)?;
        imported += 1;
    }
    Ok(Some(navicat::ImportReport { imported, skipped }))
}

/// Blank secrets fall back to the stored ones, so editing a saved connection
/// can be tested without retyping its password.
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    profile: ConnectionProfile,
    secrets: Secrets,
) -> CmdResult<String> {
    let secrets = if profile.id.is_empty() {
        secrets
    } else {
        secrets.merged_over(connections::load_secrets(&profile.id))
    };
    state.sessions.test(&profile, &secrets).await.map_err(err)
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, id: String) -> CmdResult<SessionInfo> {
    let profile = state.store.get(&id).map_err(err)?;
    let secrets = connections::load_secrets(&id);
    state.sessions.connect(profile, secrets).await.map_err(err)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    state.sessions.disconnect(&session).await;
    Ok(())
}

#[tauri::command]
pub async fn switch_database(
    state: State<'_, AppState>,
    session: String,
    database: String,
) -> CmdResult<String> {
    state
        .sessions
        .switch_database(&session, &database)
        .await
        .map_err(err)
}

macro_rules! driver {
    ($state:expr, $session:expr) => {
        $state
            .sessions
            .get(&$session)
            .await
            .map_err(err)?
            .driver()
            .await
            .map_err(err)?
    };
}

macro_rules! backend {
    ($state:expr, $session:expr) => {
        $state
            .sessions
            .get(&$session)
            .await
            .map_err(err)?
            .backend()
            .await
    };
}

#[tauri::command]
pub async fn list_databases(state: State<'_, AppState>, session: String) -> CmdResult<Vec<String>> {
    match backend!(state, session) {
        Backend::Sql(d) => d.list_databases().await.map_err(err),
        Backend::Redis(r) => Ok(r.databases().await),
        // MongoDB databases appear as schemas in the sidebar.
        Backend::Mongo(_) => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn list_schemas(state: State<'_, AppState>, session: String) -> CmdResult<Vec<String>> {
    match backend!(state, session) {
        Backend::Sql(d) => d.list_schemas().await.map_err(err),
        Backend::Mongo(m) => m.databases().await.map_err(err),
        Backend::Redis(_) => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn list_tables(
    state: State<'_, AppState>,
    session: String,
    schema: String,
) -> CmdResult<Vec<TableInfo>> {
    match backend!(state, session) {
        Backend::Sql(d) => d.list_tables(&schema).await.map_err(err),
        Backend::Mongo(m) => m.collections(&schema).await.map_err(err),
        Backend::Redis(_) => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn list_routines(
    state: State<'_, AppState>,
    session: String,
    schema: String,
) -> CmdResult<Vec<RoutineInfo>> {
    match backend!(state, session) {
        Backend::Sql(d) => d.list_routines(&schema).await.map_err(err),
        _ => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn routine_definition(
    state: State<'_, AppState>,
    session: String,
    schema: String,
    kind: String,
    id: String,
) -> CmdResult<String> {
    driver!(state, session)
        .routine_definition(&schema, &kind, &id)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn table_structure(
    state: State<'_, AppState>,
    session: String,
    table: TableRef,
) -> CmdResult<TableStructure> {
    driver!(state, session)
        .table_structure(&table)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn fetch_rows(
    state: State<'_, AppState>,
    session: String,
    request: RowsRequest,
) -> CmdResult<ResultSet> {
    driver!(state, session)
        .fetch_rows(&request)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn count_rows(
    state: State<'_, AppState>,
    session: String,
    request: RowsRequest,
) -> CmdResult<u64> {
    driver!(state, session)
        .count_rows(&request)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn execute(
    state: State<'_, AppState>,
    session: String,
    sql: String,
    max_rows: usize,
    query_id: String,
) -> CmdResult<ExecuteResult> {
    driver!(state, session)
        .execute(&sql, max_rows.max(1), &query_id)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn cancel_query(
    state: State<'_, AppState>,
    session: String,
    query_id: String,
) -> CmdResult<()> {
    driver!(state, session).cancel(&query_id).await.map_err(err)
}

#[tauri::command]
pub async fn preview_changes(
    state: State<'_, AppState>,
    session: String,
    table: TableRef,
    changes: Vec<RowChange>,
) -> CmdResult<String> {
    driver!(state, session)
        .preview_changes(&table, &changes)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn apply_changes(
    state: State<'_, AppState>,
    session: String,
    table: TableRef,
    changes: Vec<RowChange>,
) -> CmdResult<u64> {
    driver!(state, session)
        .apply_changes(&table, &changes)
        .await
        .map_err(err)
}

/// Open (or, with `create`, save-as) dialog for a file path.
#[tauri::command]
pub async fn pick_file(app: AppHandle, create: bool) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    let done = move |p: Option<tauri_plugin_dialog::FilePath>| {
        let _ = tx.send(p.and_then(|p| p.into_path().ok()));
    };
    if create {
        dialog.set_file_name("database.sqlite").save_file(done);
    } else {
        dialog.pick_file(done);
    }
    let path = rx.await.ok().flatten()?;
    if create && !path.exists() {
        // An empty file is a valid, empty SQLite database.
        std::fs::File::create(&path).ok()?;
    }
    Some(path.display().to_string())
}

#[tauri::command]
pub async fn redis_scan(
    state: State<'_, AppState>,
    session: String,
    cursor: String,
    pattern: String,
    count: u32,
) -> CmdResult<ScanPage> {
    let r = backend!(state, session).redis().map_err(err)?;
    r.scan(&cursor, &pattern, count).await.map_err(err)
}

#[tauri::command]
pub async fn redis_get(
    state: State<'_, AppState>,
    session: String,
    key: String,
) -> CmdResult<KeyValue> {
    let r = backend!(state, session).redis().map_err(err)?;
    r.get(&key).await.map_err(err)
}

/// One command, either as arguments (UI edits) or a console line.
#[tauri::command]
pub async fn redis_command(
    state: State<'_, AppState>,
    session: String,
    args: Option<Vec<String>>,
    line: Option<String>,
) -> CmdResult<serde_json::Value> {
    let r = backend!(state, session).redis().map_err(err)?;
    let args = match (args, line) {
        (Some(a), _) => a,
        (None, Some(l)) => kv::split_args(&l).map_err(err)?,
        (None, None) => return Err("no command".into()),
    };
    r.command(&args).await.map_err(err)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mongo_find(
    state: State<'_, AppState>,
    session: String,
    db: String,
    collection: String,
    filter: String,
    sort: String,
    skip: u64,
    limit: i64,
) -> CmdResult<FindResult> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.find(&db, &collection, &filter, &sort, skip, limit)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn mongo_count(
    state: State<'_, AppState>,
    session: String,
    db: String,
    collection: String,
    filter: String,
) -> CmdResult<u64> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.count(&db, &collection, &filter).await.map_err(err)
}

#[tauri::command]
pub async fn mongo_insert(
    state: State<'_, AppState>,
    session: String,
    db: String,
    collection: String,
    doc: String,
) -> CmdResult<serde_json::Value> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.insert(&db, &collection, &doc).await.map_err(err)
}

#[tauri::command]
pub async fn mongo_replace(
    state: State<'_, AppState>,
    session: String,
    db: String,
    collection: String,
    id: String,
    doc: String,
) -> CmdResult<()> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.replace(&db, &collection, &id, &doc).await.map_err(err)
}

#[tauri::command]
pub async fn mongo_delete(
    state: State<'_, AppState>,
    session: String,
    db: String,
    collection: String,
    ids: Vec<String>,
) -> CmdResult<u64> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.delete(&db, &collection, &ids).await.map_err(err)
}

#[tauri::command]
pub async fn mongo_command(
    state: State<'_, AppState>,
    session: String,
    db: String,
    command: String,
) -> CmdResult<serde_json::Value> {
    let m = backend!(state, session).mongo().map_err(err)?;
    m.command(&db, &command).await.map_err(err)
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) {
    crate::updater::check(app, true).await;
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
