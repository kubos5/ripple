//! Ripple's desktop shell: a window around the web UI, wired to the Rust core.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ripple_core::Ripple;
use ripple_core::api::{Request, UiEvent};
use tauri::http::{Response, StatusCode, header};
use tauri::ipc::Channel;
#[cfg(not(target_os = "macos"))]
use tauri::menu::{Menu, MenuItem};
#[cfg(not(target_os = "macos"))]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt as _;
use tokio::sync::broadcast::error::RecvError;

struct Core(Arc<Ripple>);

/// Whether closing the window should keep Ripple running (in the tray, or in the Dock on macOS).
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(true);

#[tauri::command]
async fn ripple(core: tauri::State<'_, Core>, req: Request) -> Result<serde_json::Value, String> {
    core.0.handle(req).await.map_err(|e| e.to_string())
}

/// Streams core events to the UI, one batch per burst.
#[tauri::command]
fn subscribe(core: tauri::State<'_, Core>, channel: Channel<Vec<UiEvent>>) {
    let mut rx = core.0.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let first = match rx.recv().await {
                Ok(e) => e,
                Err(RecvError::Lagged(_)) => UiEvent::Reset,
                Err(RecvError::Closed) => break,
            };
            let mut batch = vec![first];
            while batch.len() < 512 {
                match rx.try_recv() {
                    Ok(e) => batch.push(e),
                    Err(_) => break,
                }
            }
            if channel.send(batch).is_err() {
                break;
            }
        }
    });
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn apply_settings(app: &AppHandle, s: &ripple_core::model::Settings) {
    CLOSE_TO_TRAY.store(s.close_to_tray, Ordering::Relaxed);
    let auto = app.autolaunch();
    if auto.is_enabled().unwrap_or(false) != s.autostart {
        let _ = if s.autostart { auto.enable() } else { auto.disable() };
    }
}

/// Reacts to core events that need the operating system: notifications, badges, settings.
fn watch(app: AppHandle, core: Arc<Ripple>) {
    apply_settings(&app, &core.app().settings());
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(UiEvent::Notify { title, body, .. }) => {
                    let focused = app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false));
                    if !focused {
                        let mut n = app.notification().builder().title(title).body(body);
                        if core.app().settings().notification_sound {
                            n = n.sound("default");
                        }
                        let _ = n.show();
                    }
                }
                Ok(UiEvent::Badge { count }) => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.set_badge_count(if count > 0 { Some(count as i64) } else { None });
                    }
                    if let Some(tray) = app.tray_by_id("main") {
                        let tip = if count > 0 { format!("Ripple · {count} unread") } else { "Ripple".to_string() };
                        let _ = tray.set_tooltip(Some(tip));
                    }
                }
                Ok(UiEvent::Settings { settings }) => apply_settings(&app, &settings),
                Ok(UiEvent::Reset) => apply_settings(&app, &core.app().settings()),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Windows and Linux get a tray icon to bring the window back. macOS has the Dock for that, so no menu bar icon.
#[cfg(not(target_os = "macos"))]
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Ripple", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Ripple", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Ripple")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .build(),
        )
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // Media is streamed straight from disk with range support, so video and audio can seek.
        .register_asynchronous_uri_scheme_protocol("media", |ctx, request, responder| {
            let core = ctx.app_handle().try_state::<Core>().map(|c| c.0.clone());
            let raw = request.uri().path().trim_start_matches('/').to_string();
            let token = percent_encoding::percent_decode_str(&raw).decode_utf8_lossy().into_owned();
            let wav = request.uri().query().is_some_and(|q| q.contains("wav"));
            let range = request.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(String::from);
            tauri::async_runtime::spawn_blocking(move || {
                let served = core.and_then(|c| c.serve_media(&token, range.as_deref(), wav).ok());
                let response = match served {
                    Some(s) => {
                        let mut r = Response::builder()
                            .status(s.status)
                            .header(header::CONTENT_TYPE, if wav { "audio/wav" } else { s.mime })
                            .header(header::ACCEPT_RANGES, "bytes")
                            .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
                            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
                        if s.status == 206 {
                            r = r.header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", s.start, s.end, s.total));
                        }
                        r.body(s.body)
                    }
                    None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()),
                };
                if let Ok(r) = response {
                    responder.respond(r);
                }
            });
        })
        .invoke_handler(tauri::generate_handler![ripple, subscribe])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let demo = std::env::var_os("RIPPLE_DEMO").is_some();
            let core = tauri::async_runtime::block_on(Ripple::open(dir, demo))?;
            app.manage(Core(core.clone()));
            watch(app.handle().clone(), core);
            #[cfg(not(target_os = "macos"))]
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && CLOSE_TO_TRAY.load(Ordering::Relaxed)
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start Ripple");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, &event, RunEvent::Ready);
    });
}
