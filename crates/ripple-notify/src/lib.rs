//! Native message notifications: UserNotifications on macOS, toasts on Windows and
//! the freedesktop notification service (D-Bus) on Linux.
//!
//! Every platform gets the same behaviour: the chat's picture and a preview of a photo
//! or sticker, a click that opens the chat, and removal once the chat has been read.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "linux", all(test, unix)))]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;

/// One message notification.
#[derive(Clone, Debug, Default)]
pub struct Note {
    /// The chat it belongs to; a click opens this chat.
    pub chat: String,
    /// Identifies the notification within its chat (usually the message id).
    pub id: String,
    pub title: String,
    pub body: String,
    /// The chat's picture (PNG).
    pub avatar: Option<PathBuf>,
    /// A preview of the photo or sticker in the message (PNG).
    pub image: Option<PathBuf>,
    pub sound: bool,
}

/// Who is sending the notifications.
#[derive(Clone, Debug, Default)]
pub struct AppInfo {
    /// Bundle identifier on macOS, AppUserModelID on Windows.
    pub id: String,
    pub name: String,
    /// The `.desktop` file name without its extension (Linux).
    pub desktop_entry: String,
    /// Icon name in the icon theme (Linux).
    pub icon_name: String,
    /// A PNG of the app icon, for the notification header on Windows.
    pub icon: Option<PathBuf>,
}

/// Called with the chat to open when a notification is clicked. On Linux the second
/// argument is the activation token that lets the window come to the front on Wayland.
pub type OnOpen = Arc<dyn Fn(String, Option<String>) + Send + Sync>;

/// Chats (by [`short_hash`]) that may have notifications up, so reading any other chat
/// costs nothing. Seeded at startup with what the system still shows from earlier runs.
pub(crate) type Known = Arc<Mutex<HashSet<String>>>;

pub struct Notifier {
    #[cfg(target_os = "macos")]
    imp: mac::Mac,
    #[cfg(windows)]
    imp: win::Win,
    #[cfg(target_os = "linux")]
    imp: linux::Linux,
    #[cfg_attr(not(any(target_os = "macos", windows, target_os = "linux")), allow(dead_code))]
    known: Known,
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
impl Notifier {
    /// Returns `None` where native notifications are unavailable: on macOS that is a
    /// binary running outside an app bundle (`tauri dev`).
    pub fn new(app: AppInfo, on_open: OnOpen) -> Option<Notifier> {
        #[cfg(target_os = "macos")]
        let imp = mac::Mac::new(on_open)?;
        #[cfg(windows)]
        let imp = win::Win::new(&app, on_open)?;
        #[cfg(target_os = "linux")]
        let imp = linux::Linux::new(&app, on_open)?;
        let _ = &app;
        let known = Known::default();
        #[cfg(any(target_os = "macos", windows))]
        imp.seed(known.clone());
        Some(Notifier { imp, known })
    }

    fn known(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.known.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks for permission to notify, where the system wants that (macOS). Harmless to repeat.
    pub fn prepare(&self) {
        #[cfg(target_os = "macos")]
        self.imp.prepare();
    }

    pub fn show(&self, note: Note) {
        self.known().insert(short_hash(&note.chat));
        self.imp.show(note);
    }

    /// Removes every notification of a chat, for instance once it has been read.
    pub fn clear_chat(&self, chat: &str) {
        if self.known().remove(&short_hash(chat)) {
            self.imp.clear_chat(chat);
        }
    }

    /// Removes one notification.
    pub fn remove(&self, chat: &str, id: &str) {
        if self.known().contains(&short_hash(chat)) {
            self.imp.remove(chat, id);
        }
    }

    pub fn clear_all(&self) {
        self.known().clear();
        self.imp.clear_all();
    }

    /// Changes the icon in the notification header (Windows).
    pub fn set_icon(&self, png: &Path) {
        #[cfg(windows)]
        self.imp.set_icon(png);
        let _ = png;
    }
}

/// Gives this process the app's AppUserModelID, which Windows uses to group its windows in
/// the taskbar and to attribute its notifications. Call before any window is created.
pub fn set_process_app_id(id: &str) {
    #[cfg(windows)]
    win::set_process_app_id(id);
    let _ = id;
}

// ───────────────────────────── shared helpers ─────────────────────────────

/// A short, stable name for a chat or message id, for systems that limit tag lengths.
pub(crate) fn short_hash(s: &str) -> String {
    // FNV-1a: stable across runs and platforms, unlike `DefaultHasher`.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A `file://` URI for an absolute path, percent-encoded so spaces and non-ASCII names survive.
pub(crate) fn file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !raw.starts_with('/') {
        out.push('/'); // Windows drive paths: file:///C:/...
    }
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Escapes text for XML (Windows toasts) and for the markup subset of freedesktop notifications.
pub(crate) fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// The toast template for a note: title, text, the chat picture in a circle where the app
/// icon would be, and the photo or sticker below the text.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn toast_xml(n: &Note) -> String {
    let mut x = String::from(r#"<toast activationType="foreground"><visual><binding template="ToastGeneric">"#);
    x.push_str(&format!(r#"<text hint-maxLines="1">{}</text>"#, escape_xml(&n.title)));
    if !n.body.is_empty() {
        x.push_str(&format!("<text>{}</text>", escape_xml(&n.body)));
    }
    if let Some(a) = &n.avatar {
        x.push_str(&format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, escape_xml(&file_uri(a))));
    }
    if let Some(i) = &n.image {
        x.push_str(&format!(r#"<image src="{}"/>"#, escape_xml(&file_uri(i))));
    }
    x.push_str("</binding></visual>");
    x.push_str(if n.sound { r#"<audio src="ms-winsoundevent:Notification.IM"/>"# } else { r#"<audio silent="true"/>"# });
    x.push_str("</toast>");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_short_and_stable() {
        assert_eq!(short_hash("447700900123@s.whatsapp.net"), short_hash("447700900123@s.whatsapp.net"));
        assert_ne!(short_hash("a"), short_hash("b"));
        assert_eq!(short_hash("x").len(), 16);
    }

    #[test]
    fn file_uris() {
        assert_eq!(file_uri(Path::new("/home/ana/My Pics/ü.png")), "file:///home/ana/My%20Pics/%C3%BC.png");
        assert_eq!(file_uri(Path::new(r"C:\Users\Ana\a b.png")), "file:///C:/Users/Ana/a%20b.png");
    }

    #[test]
    fn toast_markup_is_escaped() {
        let n = Note {
            chat: "c".into(),
            id: "1".into(),
            title: "Tom & Jerry".into(),
            body: "a < b \u{1} \"quoted\"".into(),
            avatar: Some("/tmp/a.png".into()),
            image: Some("/tmp/p q.png".into()),
            sound: false,
        };
        let x = toast_xml(&n);
        assert!(x.contains("<text hint-maxLines=\"1\">Tom &amp; Jerry</text>"));
        assert!(x.contains("<text>a &lt; b  &quot;quoted&quot;</text>"));
        assert!(x.contains(r#"placement="appLogoOverride" hint-crop="circle" src="file:///tmp/a.png""#));
        assert!(x.contains(r#"<image src="file:///tmp/p%20q.png"/>"#));
        assert!(x.contains(r#"<audio silent="true"/>"#));
        let quiet = toast_xml(&Note { body: String::new(), avatar: None, image: None, sound: true, ..n });
        assert_eq!(quiet.matches("<text").count(), 1);
        assert!(!quiet.contains("<image"));
        assert!(quiet.contains("Notification.IM"));
    }
}
