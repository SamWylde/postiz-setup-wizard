mod commands;
mod state;

use state::{SharedState, TunnelProvider};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

fn preferred_public_web_link(state: &SharedState) -> Option<String> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let tunnel_alive = s
        .tunnel_pid
        .map(commands::tunnel::is_pid_alive)
        .unwrap_or(false);

    if s.tunnel_mode == "permanent" && s.tunnel_provider == TunnelProvider::Manual {
        return s.permanent_domain.clone();
    }

    if tunnel_alive {
        return s.tunnel_url.clone().or_else(|| {
            if s.tunnel_mode == "permanent" {
                s.permanent_domain.clone()
            } else {
                None
            }
        });
    }

    None
}

fn preferred_open_url(state: &SharedState) -> Option<String> {
    preferred_public_web_link(state).or_else(|| {
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .local_url
            .clone()
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(state::AppState::default()) as SharedState)
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Another instance was launched — focus the existing window
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            // Build tray menu
            let open_postiz =
                MenuItem::with_id(app, "open_postiz", "Open Postiz", true, None::<&str>)?;
            let copy_link =
                MenuItem::with_id(app, "copy_link", "Copy Web Link", true, None::<&str>)?;
            let view_status =
                MenuItem::with_id(app, "view_status", "View Status", true, None::<&str>)?;
            let restart_postiz =
                MenuItem::with_id(app, "restart_postiz", "Restart Postiz", true, None::<&str>)?;
            let stop_postiz =
                MenuItem::with_id(app, "stop_postiz", "Stop Postiz", true, None::<&str>)?;
            let check_updates =
                MenuItem::with_id(app, "check_updates", "Check for Updates", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &open_postiz,
                    &copy_link,
                    &view_status,
                    &restart_postiz,
                    &stop_postiz,
                    &check_updates,
                    &quit,
                ],
            )?;

            let tray_icon = app
                .default_window_icon()
                .cloned()
                .ok_or("No default window icon found — check tauri.conf.json bundle.icon")?;
            let _tray = TrayIconBuilder::with_id("postiz-tray")
                .icon(tray_icon)
                .menu(&menu)
                .tooltip("Postiz Setup Wizard")
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "open_postiz" => {
                            let state: tauri::State<SharedState> = app.state();
                            let url = preferred_open_url(&state);
                            if let Some(url) = url {
                                let _ = open::that(url);
                            }
                        }
                        "copy_link" => {
                            let state: tauri::State<SharedState> = app.state();
                            let url = preferred_public_web_link(&state);
                            if let Some(url) = url {
                                let _ = app.emit("copy-to-clipboard", url.clone());
                                // Show notification feedback
                                use tauri_plugin_notification::NotificationExt;
                                let _ = app
                                    .notification()
                                    .builder()
                                    .title("Web link copied")
                                    .body(&url)
                                    .show();
                            } else {
                                use tauri_plugin_notification::NotificationExt;
                                let _ = app
                                    .notification()
                                    .builder()
                                    .title("No public web link")
                                    .body("Cloudflare is disconnected. Open Postiz locally or reconnect the web link first.")
                                    .show();
                            }
                        }
                        "view_status" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "restart_postiz" => {
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let state: tauri::State<SharedState> = app_handle.state();
                                let path = {
                                    let s = state.lock().unwrap_or_else(|e| e.into_inner());
                                    s.install_path
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().to_string())
                                };
                                if let Some(path) = path {
                                    let _ = commands::docker::repair_stack_inner(
                                        &path,
                                        &app_handle,
                                    )
                                    .await;
                                }
                            });
                        }
                        "stop_postiz" => {
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let state: tauri::State<SharedState> = app_handle.state();
                                let path = {
                                    let s = state.lock().unwrap_or_else(|e| e.into_inner());
                                    s.install_path
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().to_string())
                                };
                                if let Some(path) = path {
                                    let _ = commands::docker::stop_stack_inner(&path);
                                }
                            });
                        }
                        "check_updates" => {
                            // Show main window and emit event to trigger update check
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                            let _ = app.emit("trigger-update-check", ());
                        }
                        "quit" => {
                            // Kill only our tunnel process by PID, not all cloudflared instances
                            let state: tauri::State<SharedState> = app.state();
                            {
                                let s = state.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(pid) = s.tunnel_pid {
                                    let _ = crate::commands::silent_cmd("taskkill")
                                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                                        .output();
                                }
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();

                // Show notification on first minimize
                let app = window.app_handle();
                let state: tauri::State<SharedState> = app.state();
                let should_notify = {
                    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                    if !s.has_shown_tray_notification {
                        s.has_shown_tray_notification = true;
                        true
                    } else {
                        false
                    }
                };

                if should_notify {
                    use tauri_plugin_notification::NotificationExt;
                    let _ = app
                        .notification()
                        .builder()
                        .title("Postiz is still running")
                        .body("The app has been minimized to your system tray. Double-click the tray icon to reopen.")
                        .show();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap::scan_machine,
            commands::bootstrap::run_bootstrap,
            commands::install::prepare_install,
            commands::install::commit_install,
            commands::install::get_default_install_path,
            commands::docker::start_stack,
            commands::docker::get_stack_status,
            commands::docker::stop_stack,
            commands::docker::repair_stack,
            commands::docker::get_docker_logs,
            commands::docker::cancel_install,
            commands::docker::cancel_docker_operation,
            commands::docker::restart_and_verify,
            commands::tunnel::start_tunnel,
            commands::tunnel::stop_tunnel,
            commands::tunnel::get_tunnel_status,
            commands::tunnel::reconnect_tunnel,
            commands::env_file::stage_provider_config,
            commands::env_file::apply_provider_changes,
            commands::env_file::update_base_urls,
            commands::env_file::read_env_value,
            commands::env_file::get_saved_credentials,
            commands::env_file::save_postiz_credentials,
            commands::env_file::get_postiz_credentials,
            commands::env_file::apply_config_transaction,
            commands::resume::load_resume_state,
            commands::resume::save_resume_state,
            commands::resume::update_step,
            commands::resume::sync_tunnel_config,
            commands::resume::sync_provider_status,
            commands::snapshot::get_install_snapshot,
            commands::snapshot::validate_preflight,
            commands::diagnostics::export_diagnostics,
            commands::import::import_existing_install,
            commands::install::clean_staged_files,
            commands::install::wipe_existing_install,
            commands::secrets::generate_secrets,
            commands::updater::check_for_update,
            commands::updater::install_update,
            commands::updater::get_current_app_version,
            commands::transfer::export_clone,
            commands::transfer::validate_clone_file,
            commands::transfer::import_clone,
            commands::resume::clear_transfer_review_and_save,
            commands::upgrade::check_postiz_update,
            commands::upgrade::upgrade_postiz,
            commands::web_link::configure_managed_caddy,
            commands::web_link::disable_managed_caddy,
            commands::web_link::apply_manual_domain,
            commands::web_link::switch_to_local_only,
            commands::web_link::connect_cloudflare_zero_trust,
            commands::web_link::verify_public_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
