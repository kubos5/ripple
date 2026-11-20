//! Windows: toast notifications through the WinRT notification APIs.
//!
//! The chat's picture replaces the app logo (cropped to a circle), a photo or sticker shows
//! below the text, and a click opens the chat. Toasts are grouped per chat so they can be
//! removed from Action Center once the chat has been read.

use std::path::Path;
use std::sync::Mutex;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::{HSTRING, Result};

use crate::{AppInfo, Known, Note, OnOpen, short_hash, toast_xml};

/// A toast we showed. Keeping the object alive keeps its click handler alive.
struct Live {
    chat: String,
    toast: ToastNotification,
}

pub struct Win {
    app_id: HSTRING,
    name: String,
    on_open: OnOpen,
    live: Mutex<Vec<Live>>,
}

pub fn set_process_app_id(id: &str) {
    // SAFETY: plain Win32 call with a valid, NUL-terminated string.
    if let Err(e) = unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id)) } {
        log::warn!("could not set the app id: {e}");
    }
}

/// Registers the app id with a display name and icon. The installer's Start menu shortcut
/// does the same; this also covers portable and development runs, which have no shortcut.
fn register(id: &str, name: &str, icon: Option<&Path>) {
    let set = || -> windows_registry::Result<()> {
        let key = windows_registry::CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{id}"))?;
        key.set_string("DisplayName", name)?;
        if let Some(icon) = icon.and_then(|p| p.to_str()) {
            key.set_string("IconUri", icon)?;
        }
        Ok(())
    };
    if let Err(e) = set() {
        log::warn!("could not register for notifications: {e}");
    }
}

impl Win {
    pub fn new(app: &AppInfo, on_open: OnOpen) -> Option<Win> {
        register(&app.id, &app.name, app.icon.as_deref());
        Some(Win { app_id: HSTRING::from(&app.id), name: app.name.clone(), on_open, live: Mutex::new(Vec::new()) })
    }

    /// Notes which chats still have toasts in Action Center from an earlier run.
    pub fn seed(&self, known: Known) {
        let Ok(history) = ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&self.app_id)) else { return };
        let mut known = known.lock().unwrap_or_else(|e| e.into_inner());
        for toast in history {
            if let Ok(group) = toast.Group() {
                known.insert(group.to_string());
            }
        }
    }

    pub fn set_icon(&self, png: &Path) {
        register(&self.app_id.to_string(), &self.name, Some(png));
    }

    pub fn show(&self, n: Note) {
        if let Err(e) = self.try_show(&n) {
            log::warn!("notification not shown: {e}");
        }
    }

    fn try_show(&self, n: &Note) -> Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(n)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetGroup(&HSTRING::from(short_hash(&n.chat)))?;
        toast.SetTag(&HSTRING::from(short_hash(&n.id)))?;
        let (open, chat) = (self.on_open.clone(), n.chat.clone());
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            open(chat.clone(), None);
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&self.app_id)?.Show(&toast)?;
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.push(Live { chat: n.chat.clone(), toast });
        // Action Center keeps a limited number per app; old handles are no use.
        if live.len() > 100 {
            live.remove(0);
        }
        Ok(())
    }

    pub fn clear_chat(&self, chat: &str) {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).retain(|l| l.chat != chat);
        // By group name, so toasts from before a restart go too.
        if let Ok(h) = ToastNotificationManager::History() {
            let _ = h.RemoveGroupWithId(&HSTRING::from(short_hash(chat)), &self.app_id);
        }
    }

    pub fn remove(&self, chat: &str, id: &str) {
        if let Ok(h) = ToastNotificationManager::History() {
            let _ = h.RemoveGroupedTagWithId(&HSTRING::from(short_hash(id)), &HSTRING::from(short_hash(chat)), &self.app_id);
        }
        let tag = HSTRING::from(short_hash(id));
        self.live.lock().unwrap_or_else(|e| e.into_inner()).retain(|l| l.toast.Tag().ok().as_ref() != Some(&tag));
    }

    pub fn clear_all(&self) {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if let Ok(h) = ToastNotificationManager::History() {
            let _ = h.ClearWithId(&self.app_id);
        }
    }
}
