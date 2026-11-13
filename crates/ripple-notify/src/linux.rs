//! Linux: the freedesktop notification service over D-Bus (GNOME, KDE Plasma and others).
//!
//! The notification carries the chat's picture, or the photo or sticker when the message has
//! one (the spec has room for a single image). A click opens the chat, using the activation
//! token the service sends so the window may come to the front on Wayland.

use std::collections::HashMap;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::Connection;
use zbus::zvariant::Value;

use crate::{AppInfo, Note, OnOpen, escape_xml, file_uri};

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn activation_token(&self, id: u32, activation_token: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

pub(crate) enum Cmd {
    Show(Note),
    ClearChat(String),
    Remove(String, String),
    ClearAll,
}

pub struct Linux {
    tx: mpsc::UnboundedSender<Cmd>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Linux {
    pub fn new(app: &AppInfo, on_open: OnOpen) -> Option<Linux> {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = app.clone();
        std::thread::Builder::new()
            .name("notifications".into())
            .spawn(move || {
                async_io::block_on(async move {
                    match Connection::session().await {
                        Ok(conn) => {
                            if let Err(e) = run(conn, app, rx, on_open).await {
                                log::warn!("notifications stopped: {e}");
                            }
                        }
                        Err(e) => log::warn!("no session bus for notifications: {e}"),
                    }
                })
            })
            .ok()?;
        Some(Linux { tx })
    }

    pub fn show(&self, n: Note) {
        let _ = self.tx.send(Cmd::Show(n));
    }

    pub fn clear_chat(&self, chat: &str) {
        let _ = self.tx.send(Cmd::ClearChat(chat.to_string()));
    }

    pub fn remove(&self, chat: &str, id: &str) {
        let _ = self.tx.send(Cmd::Remove(chat.to_string(), id.to_string()));
    }

    pub fn clear_all(&self) {
        let _ = self.tx.send(Cmd::ClearAll);
    }
}

/// Serves commands and the service's signals until the sender goes away.
pub(crate) async fn run(conn: Connection, app: AppInfo, mut rx: mpsc::UnboundedReceiver<Cmd>, on_open: OnOpen) -> zbus::Result<()> {
    let proxy = NotificationsProxy::new(&conn).await?;
    let caps = proxy.get_capabilities().await.unwrap_or_default();
    // Services that read markup would swallow "<" and "&" in a message.
    let markup = caps.iter().any(|c| c == "body-markup");
    // One stream for all signals keeps them in order: a click's activation token comes
    // just before the click itself.
    let mut signals = proxy.inner().receive_all_signals().await?;
    // Service id → (chat, note id), for clicks and removal.
    let mut shown: HashMap<u32, (String, String)> = HashMap::new();
    let mut activation: HashMap<u32, String> = HashMap::new();
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                let gone: Vec<u32> = match cmd {
                    Cmd::Show(n) => {
                        match show(&proxy, &app, &n, markup).await {
                            Ok(id) => {
                                shown.insert(id, (n.chat, n.id));
                            }
                            Err(e) => log::warn!("notification not shown: {e}"),
                        }
                        continue;
                    }
                    Cmd::ClearChat(chat) => shown.iter().filter(|(_, (c, _))| *c == chat).map(|(k, _)| *k).collect(),
                    Cmd::Remove(chat, id) => shown.iter().filter(|(_, (c, i))| *c == chat && *i == id).map(|(k, _)| *k).collect(),
                    Cmd::ClearAll => shown.keys().copied().collect(),
                };
                for id in gone {
                    shown.remove(&id);
                    activation.remove(&id);
                    let _ = proxy.close_notification(id).await;
                }
            }
            Some(msg) = signals.next() => {
                if let Some(s) = ActivationToken::from_message(msg.clone()) {
                    if let Ok(a) = s.args() {
                        activation.insert(a.id, a.activation_token);
                    }
                } else if let Some(s) = ActionInvoked::from_message(msg.clone()) {
                    if let Ok(a) = s.args()
                        && let Some((chat, _)) = shown.get(&a.id)
                    {
                        on_open(chat.clone(), activation.remove(&a.id));
                    }
                } else if let Some(s) = NotificationClosed::from_message(msg)
                    && let Ok(a) = s.args()
                {
                    shown.remove(&a.id);
                    activation.remove(&a.id);
                }
            }
        }
    }
    Ok(())
}

async fn show(proxy: &NotificationsProxy<'_>, app: &AppInfo, n: &Note, markup: bool) -> zbus::Result<u32> {
    let body = if markup { escape_xml(&n.body) } else { n.body.clone() };
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(app.desktop_entry.as_str()));
    hints.insert("category", Value::from("im.received"));
    hints.insert("urgency", Value::U8(1));
    if n.sound {
        hints.insert("sound-name", Value::from("message-new-instant"));
    } else {
        hints.insert("suppress-sound", Value::from(true));
    }
    // One image only: the photo or sticker when there is one, otherwise the chat's picture.
    let image = n.image.as_deref().or(n.avatar.as_deref()).map(file_uri);
    if let Some(uri) = &image {
        hints.insert("image-path", Value::from(uri.as_str()));
    }
    // "default" is the action for a click on the notification itself.
    proxy.notify(&app.name, 0, &app.icon_name, &n.title, &body, &["default", "Open"], hints, -1).await
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::OwnedValue;

    use super::*;

    type Call = (String, String, String, Vec<String>, HashMap<String, OwnedValue>);
    type Opened = Arc<Mutex<Vec<(String, Option<String>)>>>;

    #[derive(Default, Clone)]
    struct Seen {
        calls: Arc<Mutex<Vec<Call>>>,
        closed: Arc<Mutex<Vec<u32>>>,
    }

    struct Service {
        seen: Seen,
        next: u32,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Service {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &mut self,
            app_name: String,
            _replaces_id: u32,
            _app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            self.next += 1;
            self.seen.calls.lock().unwrap().push((app_name, summary, body, actions, hints));
            self.next
        }

        fn close_notification(&mut self, id: u32) {
            self.seen.closed.lock().unwrap().push(id);
        }

        fn get_capabilities(&self) -> Vec<String> {
            vec!["body".into(), "body-markup".into(), "actions".into()]
        }

        #[zbus(signal)]
        async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn activation_token(emitter: &SignalEmitter<'_>, id: u32, activation_token: &str) -> zbus::Result<()>;
    }

    fn wait(what: &str, ok: impl Fn() -> bool) {
        let start = Instant::now();
        while !ok() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn shows_opens_and_clears() {
        let (a, b) = UnixStream::pair().unwrap();
        let seen = Seen::default();
        let service = Service { seen: seen.clone(), next: 0 };
        let (server, client) = async_io::block_on(async {
            let guid = zbus::Guid::generate();
            let server = zbus::connection::Builder::async_io_unix_stream(a)
                .server(guid)
                .unwrap()
                .p2p()
                .serve_at("/org/freedesktop/Notifications", service)
                .unwrap()
                .build();
            let client = zbus::connection::Builder::async_io_unix_stream(b).p2p().build();
            let (s, c) = futures_util::future::join(server, client).await;
            (s.unwrap(), c.unwrap())
        });

        let opened: Opened = Arc::default();
        let sink = opened.clone();
        let on_open: OnOpen = Arc::new(move |chat, token| sink.lock().unwrap().push((chat, token)));
        let (tx, rx) = mpsc::unbounded_channel();
        let app = AppInfo { name: "Ripple".into(), desktop_entry: "Ripple".into(), icon_name: "ripple".into(), ..Default::default() };
        std::thread::spawn(move || async_io::block_on(run(client, app, rx, on_open)));

        tx.send(Cmd::Show(Note {
            chat: "dad@s.whatsapp.net".into(),
            id: "M1".into(),
            title: "Dad".into(),
            body: "Is 3 < 4 & 5?".into(),
            avatar: Some("/tmp/avatar.png".into()),
            image: Some("/tmp/my sticker.png".into()),
            sound: false,
        }))
        .unwrap();
        tx.send(Cmd::Show(Note { chat: "mum@s.whatsapp.net".into(), id: "M2".into(), title: "Mum".into(), sound: true, ..Default::default() }))
            .unwrap();
        wait("two notifications", || seen.calls.lock().unwrap().len() == 2);
        {
            let calls = seen.calls.lock().unwrap();
            let (app_name, summary, body, actions, hints) = &calls[0];
            assert_eq!((app_name.as_str(), summary.as_str()), ("Ripple", "Dad"));
            assert_eq!(body, "Is 3 &lt; 4 &amp; 5?");
            assert_eq!(actions, &["default", "Open"]);
            let text = |k: &str| hints.get(k).and_then(|v| String::try_from(v.clone()).ok());
            // The sticker wins over the avatar: the spec has room for one image.
            assert_eq!(text("image-path").as_deref(), Some("file:///tmp/my%20sticker.png"));
            assert_eq!(text("desktop-entry").as_deref(), Some("Ripple"));
            assert!(hints.contains_key("suppress-sound"));
            assert_eq!(text("sound-name"), None);
            let (.., hints) = &calls[1];
            assert_eq!(hints.get("image-path"), None);
            assert!(hints.contains_key("sound-name"));
        }

        // A click: the service sends the activation token, then the action.
        async_io::block_on(async {
            let iface = server.object_server().interface::<_, Service>("/org/freedesktop/Notifications").await.unwrap();
            Service::activation_token(iface.signal_emitter(), 1, "token-123").await.unwrap();
            Service::action_invoked(iface.signal_emitter(), 1, "default").await.unwrap();
        });
        wait("the chat to open", || !opened.lock().unwrap().is_empty());
        assert_eq!(opened.lock().unwrap()[0], ("dad@s.whatsapp.net".into(), Some("token-123".into())));

        // Reading Dad's chat closes only Dad's notification.
        tx.send(Cmd::ClearChat("dad@s.whatsapp.net".into())).unwrap();
        wait("the close", || !seen.closed.lock().unwrap().is_empty());
        tx.send(Cmd::Remove("mum@s.whatsapp.net".into(), "nope".into())).unwrap();
        tx.send(Cmd::ClearAll).unwrap();
        wait("the second close", || seen.closed.lock().unwrap().len() == 2);
        assert_eq!(*seen.closed.lock().unwrap(), vec![1, 2]);
    }
}
