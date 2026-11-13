//! macOS: the UserNotifications framework.
//!
//! macOS always shows the app's own icon on the left of a notification (only apps with Apple's
//! communication-notification entitlement may replace it), so the chat's picture, or the photo
//! or sticker when there is one, appears as the thumbnail on the right. Notifications are
//! grouped per chat in Notification Center.

use std::path::Path;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAttachment,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::{Known, Note, OnOpen, short_hash};

static ON_OPEN: OnceLock<OnOpen> = OnceLock::new();
static ASKED: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class has no Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "RippleNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Ripple only posts while its window is in the background, so a notification that
        /// arrives while the app is frontmost (window hidden or minimised) is still shown.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            done.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        /// A click on the notification: open its chat.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            done: &block2::DynBlock<dyn Fn()>,
        ) {
            let chat = response.notification().request().content().threadIdentifier().to_string();
            if let Some(open) = ON_OPEN.get()
                && !chat.is_empty()
            {
                open(chat, None);
            }
            done.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: plain NSObject initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Mac;

fn center() -> Retained<UNUserNotificationCenter> {
    UNUserNotificationCenter::currentNotificationCenter()
}

fn request_id(chat: &str, id: &str) -> String {
    format!("{chat}/{id}")
}

impl Mac {
    pub fn new(on_open: OnOpen) -> Option<Mac> {
        // UserNotifications throws for a process that is not an app bundle (as under `tauri dev`).
        let bundle = NSBundle::mainBundle();
        if bundle.bundleIdentifier().is_none() || !bundle.bundlePath().to_string().ends_with(".app") {
            return None;
        }
        let _ = ON_OPEN.set(on_open);
        let delegate = Delegate::new();
        center().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly; this one lives as long as the app.
        std::mem::forget(delegate);
        Some(Mac)
    }

    /// Notes which chats still have notifications from an earlier run.
    pub fn seed(&self, known: Known) {
        let done = RcBlock::new(move |list: NonNull<NSArray<UNNotification>>| {
            // SAFETY: the framework passes a valid array.
            let list = unsafe { list.as_ref() };
            let mut known = known.lock().unwrap_or_else(|e| e.into_inner());
            for n in list.iter() {
                known.insert(short_hash(&n.request().content().threadIdentifier().to_string()));
            }
        });
        center().getDeliveredNotificationsWithCompletionHandler(&done);
    }

    pub fn prepare(&self) {
        // macOS asks the person once; later requests just report their answer.
        if ASKED.swap(true, Ordering::Relaxed) {
            return;
        }
        let done = RcBlock::new(|granted: Bool, err: *mut NSError| {
            // SAFETY: the framework passes either null or a valid error.
            let why = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
            log::info!("notifications allowed: {} {why}", granted.as_bool());
        });
        center().requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );
    }

    pub fn show(&self, n: Note) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&n.title));
        content.setBody(&NSString::from_str(&n.body));
        // Groups the chat's notifications, and tells a click which chat to open.
        content.setThreadIdentifier(&NSString::from_str(&n.chat));
        if n.sound {
            content.setSound(Some(&UNNotificationSound::defaultSound()));
        }
        if let Some(pic) = n.image.as_deref().or(n.avatar.as_deref()).and_then(attachment) {
            content.setAttachments(&NSArray::from_retained_slice(&[pic]));
        }
        let id = NSString::from_str(&request_id(&n.chat, &n.id));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let done = RcBlock::new(|err: *mut NSError| {
            // SAFETY: the framework passes either null or a valid error.
            if let Some(e) = unsafe { err.as_ref() } {
                log::warn!("notification not shown: {}", e.localizedDescription());
            }
        });
        center().addNotificationRequest_withCompletionHandler(&request, Some(&done));
    }

    pub fn clear_chat(&self, chat: &str) {
        let chat = chat.to_string();
        // Asks the system rather than remembering, so notifications from before a restart go too.
        let done = RcBlock::new(move |list: NonNull<NSArray<UNNotification>>| {
            // SAFETY: the framework passes a valid array.
            let list = unsafe { list.as_ref() };
            let ids: Vec<Retained<NSString>> = list
                .iter()
                .map(|n| n.request())
                .filter(|r| r.content().threadIdentifier().to_string() == chat)
                .map(|r| r.identifier())
                .collect();
            if !ids.is_empty() {
                center().removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&ids));
            }
        });
        center().getDeliveredNotificationsWithCompletionHandler(&done);
    }

    pub fn remove(&self, chat: &str, id: &str) {
        let id = NSString::from_str(&request_id(chat, id));
        center().removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&[id]));
    }

    pub fn clear_all(&self) {
        center().removeAllDeliveredNotifications();
    }
}

/// The system moves attachment files into its own store, so it gets a copy.
fn attachment(src: &Path) -> Option<Retained<UNNotificationAttachment>> {
    let dir = std::env::temp_dir().join("ripple-notifications");
    std::fs::create_dir_all(&dir).ok()?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let copy = dir.join(format!("{nonce}.{ext}"));
    std::fs::copy(src, &copy).ok()?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(copy.to_str()?));
    // SAFETY: no options dictionary is passed.
    let made = unsafe {
        UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(&NSString::from_str("picture"), &url, None)
    };
    match made {
        Ok(a) => Some(a),
        Err(e) => {
            log::warn!("notification picture refused: {}", e.localizedDescription());
            let _ = std::fs::remove_file(&copy);
            None
        }
    }
}
