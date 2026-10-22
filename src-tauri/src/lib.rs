mod catalog;
mod commands;
mod db;
mod diagnostics;
mod hexutil;
mod housekeeping;
mod jsonutil;
mod paths;
mod profile_input;
mod scan;
mod secret;
mod session;
mod settings;
mod state;
mod steam;
mod updater;

use std::time::Duration;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub fn run() {
    let paths = paths::AppPaths::detect();
    let settings = settings::Settings::load(&paths.settings_file());
    let theme = settings.theme;
    housekeeping::tidy_up(&paths);
    let app_state = state::AppState::new(paths.clone(), settings);
    scan::import_legacy_summary(&paths, &app_state);

    tauri::Builder::default()
        // Opening the app while it's already running brings the open window
        // forward instead of starting a second copy. This must be the first plugin.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::app_ready,
            commands::get_app_info,
            commands::get_settings,
            commands::update_preferences,
            commands::save_api_key,
            commands::set_profile,
            commands::start_scan,
            commands::pause_scan,
            commands::get_scan_status,
            commands::get_last_scan,
            commands::get_games,
            commands::get_diagnostics_plan,
            commands::run_diagnostics,
            commands::read_report,
            commands::reveal_report,
            commands::open_link,
            commands::get_update_info,
            commands::check_for_updates,
            commands::install_update,
            commands::get_versions,
            commands::install_version,
            commands::sign_in,
            commands::sign_out,
        ])
        .setup(move |app| {
            // The window starts hidden and the interface shows it once it has drawn,
            // so there is no white flash on launch. WebView2's own files go in the
            // data folder too, keeping everything in one place.
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Steam Collection Tracker")
                    .inner_size(1240.0, 800.0)
                    .min_inner_size(960.0, 620.0)
                    .theme(Some(theme.window_theme()))
                    .visible(false)
                    .data_directory(paths.webview_dir())
                    .build()?;

            // Safety net: show the window even if the interface never reports in.
            let fallback = window.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(4)).await;
                let _ = fallback.show();
            });

            updater::spawn_startup_check(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Steam Collection Tracker");
}
