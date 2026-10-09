mod commands;
mod menu;
pub mod connections;
pub mod db;
pub mod docdb;
pub mod kv;
pub mod navicat;
pub mod session;
pub mod ssh;
mod updater;

use tauri::{Manager, RunEvent};

pub struct AppState {
    store: connections::Store,
    sessions: session::Sessions,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.set_menu(menu::build(app.handle())?)?;
            app.on_menu_event(|app, event| menu::handle(app, event.id().as_ref()));
            let dir = app.path().app_config_dir()?;
            app.manage(AppState {
                store: connections::Store::new(&dir),
                sessions: session::Sessions::new(dir),
            });
            // Debug builds are not released, so they have nothing to update to.
            if !cfg!(debug_assertions) {
                updater::start_background_checks(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::list_groups,
            commands::list_group_colors,
            commands::set_group_color,
            commands::create_group,
            commands::rename_group,
            commands::delete_group,
            commands::move_connection,
            commands::test_connection,
            commands::import_navicat,
            commands::connect,
            commands::disconnect,
            commands::switch_database,
            commands::list_databases,
            commands::list_schemas,
            commands::list_tables,
            commands::list_routines,
            commands::routine_definition,
            commands::table_structure,
            commands::fetch_rows,
            commands::count_rows,
            commands::execute,
            commands::cancel_query,
            commands::preview_changes,
            commands::apply_changes,
            commands::pick_file,
            commands::save_sql_file,
            commands::open_sql_file,
            commands::check_updates,
            commands::app_version,
            commands::redis_scan,
            commands::redis_get,
            commands::redis_command,
            commands::mongo_find,
            commands::mongo_count,
            commands::mongo_insert,
            commands::mongo_replace,
            commands::mongo_delete,
            commands::mongo_command,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tablory");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let state = app.state::<AppState>();
            tauri::async_runtime::block_on(state.sessions.close_all());
        }
    });
}
