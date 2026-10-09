//! The native menu bar. App actions are sent to the frontend as a `menu`
//! event carrying the item id; About and updates are handled here.

use tauri::menu::{AboutMetadataBuilder, Menu, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Runtime};

use crate::updater;

const REPO: &str = "https://github.com/ashafizullah/tablory";

/// The webview handles the shortcuts itself. macOS gets real key equivalents
/// (the webview sees the key first); elsewhere the shortcut is only shown,
/// since a native accelerator would fire as well as the page's own handler.
fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    label: &str,
    shortcut: Option<&str>,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(id, label);
    if let Some(keys) = shortcut {
        if cfg!(target_os = "macos") {
            b = b.accelerator(keys);
        } else {
            let keys = keys.replace("CmdOrCtrl", "Ctrl");
            b = MenuItemBuilder::with_id(id, format!("{label}\t{keys}"));
        }
    }
    b.build(app)
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let info = app.package_info();
    let about = AboutMetadataBuilder::new()
        .name(Some(info.name.clone()))
        .version(Some(info.version.to_string()))
        .comments(Some(
            "Free, open-source, native database manager".to_string(),
        ))
        .copyright(Some("MIT License".to_string()))
        .license(Some("MIT".to_string()))
        .website(Some(REPO.to_string()))
        .website_label(Some("GitHub".to_string()))
        .build();
    let check_updates = item(app, "check-updates", "Check for Updates…", None)?;

    let mut file = SubmenuBuilder::new(app, "File")
        .item(&item(app, "new-query", "New Query", Some("CmdOrCtrl+T"))?)
        .item(&item(app, "open-file", "Open SQL File…", Some("CmdOrCtrl+O"))?)
        .separator()
        .item(&item(app, "save", "Save", Some("CmdOrCtrl+S"))?)
        .item(&item(app, "save-as", "Save As…", Some("CmdOrCtrl+Shift+S"))?)
        .separator()
        .item(&item(app, "close-tab", "Close Tab", Some("CmdOrCtrl+W"))?)
        .separator()
        .item(&item(app, "manage", "Manage Connections…", None)?)
        .item(&item(app, "disconnect", "Disconnect", Some("CmdOrCtrl+K"))?);
    if !cfg!(target_os = "macos") {
        file = file.separator().text("quit", "Exit");
    }
    let file = file.build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item(
            app,
            "toggle-rail",
            "Show/Hide Connections",
            Some("CmdOrCtrl+B"),
        )?)
        .build()?;

    let mut help = SubmenuBuilder::new(app, "Help");
    if !cfg!(target_os = "macos") {
        help = help
            .about_with_text("About Tablory", Some(about.clone()))
            .item(&check_updates)
            .separator();
    }
    let help = help
        .text("website", "Tablory on GitHub")
        .text("issues", "Report an Issue")
        .text("releases", "Release Notes")
        .build()?;

    if cfg!(target_os = "macos") {
        // The app menu, and Edit so ⌘C/⌘V/⌘A reach the webview.
        let app_menu = SubmenuBuilder::new(app, info.name.clone())
            .about(Some(about))
            .item(&check_updates)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        let edit = SubmenuBuilder::new(app, "Edit")
            .undo()
            .redo()
            .separator()
            .cut()
            .copy()
            .paste()
            .select_all()
            .build()?;
        Menu::with_items(app, &[&app_menu, &file, &edit, &view, &help])
    } else {
        Menu::with_items(app, &[&file, &view, &help])
    }
}

pub fn handle(app: &AppHandle, id: &str) {
    match id {
        "check-updates" => {
            let app = app.clone();
            // The update dialogs block, so keep them off the main thread.
            std::thread::spawn(move || {
                tauri::async_runtime::block_on(updater::check(app, true))
            });
        }
        "quit" => app.exit(0),
        "website" => open_url(REPO),
        "issues" => open_url(&format!("{REPO}/issues")),
        "releases" => open_url(&format!("{REPO}/releases")),
        _ => {
            let _ = app.emit("menu", id);
        }
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    let _ = cmd;
}
