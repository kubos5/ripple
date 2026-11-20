//! Ripple's desktop shell: a window around the web UI, wired to the Rust core.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod badge;
mod icons;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ripple_core::Ripple;
use ripple_core::api::{Request, UiEvent, call_note_id};
use ripple_core::model::{Content, bare};
use ripple_notify::{AppInfo, Note, Notifier};
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

/// Brings the window forward for a notification click. On Wayland the window may only take
/// focus with the activation token the notification service handed over.
fn activate(app: &AppHandle, token: Option<String>) {
    #[cfg(target_os = "linux")]
    if let (Some(token), Some(w)) = (&token, app.get_webview_window("main"))
        && let Ok(gtk_window) = w.gtk_window()
    {
        use gtk::prelude::GtkWindowExt as _;
        gtk_window.set_startup_id(token);
    }
    let _ = token;
    show_main(app);
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn apply_settings(app: &AppHandle, s: &ripple_core::model::Settings, notes: Option<&Notifier>) {
    CLOSE_TO_TRAY.store(s.close_to_tray, Ordering::Relaxed);
    if icons::apply(app, s.green_icon)
        && let (Some(n), Some(icon)) = (notes, notification_icon(app, s.green_icon))
    {
        n.set_icon(&icon);
    }
    if s.notifications
        && let Some(n) = notes
    {
        n.prepare();
    }
    let auto = app.autolaunch();
    if auto.is_enabled().unwrap_or(false) != s.autostart {
        let _ = if s.autostart { auto.enable() } else { auto.disable() };
    }
}

fn window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false))
}

/// The app icon as a file, for the notification header on Windows (written per colour).
fn notification_icon(app: &AppHandle, green: bool) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let dir = app.path().app_data_dir().ok()?;
    let path = dir.join(if green { "notification-icon-green.png" } else { "notification-icon.png" });
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(&path, icons::png(green)).ok()?;
    Some(path)
}

/// The system's notification service, wired to open a chat when a notification is clicked.
fn notifier(app: &AppHandle, core: &Arc<Ripple>) -> Option<Notifier> {
    let info = AppInfo {
        id: app.config().identifier.clone(),
        name: "Ripple".into(),
        desktop_entry: "Ripple".into(),
        icon_name: "ripple".into(),
        icon: notification_icon(app, core.app().settings().green_icon),
    };
    let (app, core) = (app.clone(), core.clone());
    Notifier::new(
        info,
        Arc::new(move |chat, token| {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || activate(&handle, token));
            core.reveal(&chat);
        }),
    )
}

/// Reacts to core events that need the operating system: notifications, badges, settings.
fn watch(app: AppHandle, core: Arc<Ripple>, notes: Option<Notifier>) {
    apply_settings(&app, &core.app().settings(), notes.as_ref());
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            match ev {
                UiEvent::Notify { chat, id, title, body, avatar, image } => {
                    // Like WhatsApp: no banners while Ripple is the window in front.
                    if window_focused(&app) {
                        continue;
                    }
                    let sound = core.app().settings().notification_sound;
                    match &notes {
                        Some(n) => n.show(Note {
                            chat,
                            id,
                            title,
                            body,
                            avatar: avatar.map(PathBuf::from),
                            image: image.map(PathBuf::from),
                            sound,
                        }),
                        // macOS outside an app bundle (`tauri dev`): a plain notification.
                        None => {
                            let mut n = app.notification().builder().title(title).body(body);
                            if sound {
                                n = n.sound("default");
                            }
                            let _ = n.show();
                        }
                    }
                }
                // A chat with nothing unread has been read, here or on another device.
                UiEvent::Chat { chat } if chat.unread == 0 => {
                    if let Some(n) = &notes {
                        n.clear_chat(&chat.id);
                    }
                }
                UiEvent::ChatRemoved { id } => {
                    if let Some(n) = &notes {
                        n.clear_chat(&id);
                    }
                }
                UiEvent::Message { message, .. } if matches!(message.content, Content::Deleted) => {
                    if let Some(n) = &notes {
                        n.remove(&message.chat, &message.id);
                    }
                }
                UiEvent::MessageRemoved { chat, id } => {
                    if let Some(n) = &notes {
                        n.remove(&chat, &id);
                    }
                }
                // The call stopped ringing: answered elsewhere, declined or missed.
                UiEvent::Call { id, from, ringing: false, .. } => {
                    if let Some(n) = &notes {
                        n.remove(&bare(&from), &call_note_id(&id));
                    }
                }
                UiEvent::Badge { count } => {
                    if let Some(w) = app.get_webview_window("main") {
                        // Windows has no badge for desktop apps; it gets a drawn overlay instead.
                        #[cfg(windows)]
                        let _ = w.set_overlay_icon(badge::overlay(count).map(|(rgba, w, h)| tauri::image::Image::new_owned(rgba, w, h)));
                        #[cfg(not(windows))]
                        let _ = w.set_badge_count(if count > 0 { Some(count as i64) } else { None });
                    }
                    if let Some(tray) = app.tray_by_id("main") {
                        let tip = if count > 0 { format!("Ripple · {count} unread") } else { "Ripple".to_string() };
                        let _ = tray.set_tooltip(Some(tip));
                    }
                }
                UiEvent::Settings { settings } => apply_settings(&app, &settings, notes.as_ref()),
                // Logged out, or into or out of the demo: none of the old notifications apply.
                UiEvent::Reset => {
                    if let Some(n) = &notes {
                        n.clear_all();
                    }
                    apply_settings(&app, &core.app().settings(), notes.as_ref());
                }
                _ => {}
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
    // Before any window exists, so the taskbar and notifications agree on who Ripple is.
    ripple_notify::set_process_app_id("app.ripple.desktop");
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
            // The tray first, so the settings applied next can recolour it.
            #[cfg(not(target_os = "macos"))]
            build_tray(app.handle())?;
            let notes = notifier(app.handle(), &core);
            watch(app.handle().clone(), core, notes);
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
