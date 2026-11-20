//! The app icon where it shows while Ripple runs: the window and tray icon on Windows and
//! Linux, the Dock on macOS. Grey circles by default, green when the setting asks for it.

use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(not(target_os = "macos"))]
use tauri::Manager as _;
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri::image::Image;

const LARGE: &[u8] = include_bytes!("../icons/128x128@2x.png");
const LARGE_GREEN: &[u8] = include_bytes!("../icons/green/256x256.png");
#[cfg(not(target_os = "macos"))]
const TRAY: &[u8] = include_bytes!("../icons/32x32.png");
#[cfg(not(target_os = "macos"))]
const TRAY_GREEN: &[u8] = include_bytes!("../icons/green/32x32.png");
#[cfg(target_os = "macos")]
const DOCK_GREEN: &[u8] = include_bytes!("../icons/green/macos.png");

/// What is showing: 0 before the first call, then 1 for the default icon and 2 for green.
static SHOWING: AtomicU8 = AtomicU8::new(0);

/// Shows the default or green icon. Returns whether anything changed.
pub fn apply(app: &AppHandle, green: bool) -> bool {
    let want = if green { 2 } else { 1 };
    let was = SHOWING.swap(want, Ordering::Relaxed);
    if was == want {
        return false;
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Also replaces the small default window icon on Linux with a sharp one.
        if let (Some(w), Ok(img)) = (app.get_webview_window("main"), Image::from_bytes(if green { LARGE_GREEN } else { LARGE })) {
            let _ = w.set_icon(img);
        }
        if let (Some(tray), Ok(img)) = (app.tray_by_id("main"), Image::from_bytes(if green { TRAY_GREEN } else { TRAY })) {
            let _ = tray.set_icon(Some(img));
        }
    }
    let _ = &app;
    #[cfg(target_os = "macos")]
    {
        // At startup with the default icon there is nothing to change.
        if !(was == 0 && !green) {
            let _ = app.run_on_main_thread(move || dock_icon(green));
        }
    }
    true
}

/// The icon as a PNG, for the notification header on Windows.
pub fn png(green: bool) -> &'static [u8] {
    if green { LARGE_GREEN } else { LARGE }
}

/// Swaps the running app's Dock icon. `false` goes back to the icon in the bundle.
#[cfg(target_os = "macos")]
fn dock_icon(green: bool) {
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let image = green.then(|| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(DOCK_GREEN))).flatten();
    // SAFETY: called on the main thread; `None` restores the bundle's icon.
    unsafe { app.setApplicationIconImage(image.as_deref()) };
}
