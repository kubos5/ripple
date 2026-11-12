//! The heart of Ripple: owns the store, answers UI requests, applies what the
//! network reports, and tells the UI what changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::api::*;
use crate::media::{self, MediaDir};
use crate::model::*;
use crate::store::Store;

const PAGE: usize = 60;
/// How many recently sent stickers the picker shows.
pub const RECENT_STICKERS: usize = 48;

pub struct App {
    pub store: Store,
    pub media: MediaDir,
    pub root: PathBuf,
    pub demo: bool,
    events: broadcast::Sender<UiEvent>,
    net: RwLock<Arc<dyn Network>>,
    session: Mutex<Session>,
    me: RwLock<Jid>,
    chats: Mutex<HashMap<Jid, Chat>>,
    active: Mutex<Option<Jid>>,
    focused: AtomicBool,
    settings: RwLock<Settings>,
    badge: AtomicU32,
    history_asked: Mutex<HashMap<Jid, i64>>,
    /// Library stickers being downloaded (or that failed this session).
    sticker_jobs: Mutex<HashSet<String>>,
    #[cfg(feature = "voice")]
    recorder: Mutex<Option<crate::voice::Recorder>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn new_message_id() -> String {
    format!("3EB0{}", media::random_hex(9).to_uppercase())
}

fn val<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

impl App {
    pub fn new(root: PathBuf, events: broadcast::Sender<UiEvent>, demo: bool) -> Result<Arc<App>> {
        std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        let store = Store::open(&root.join("app.db"))?;
        let media = MediaDir::new(root.join("media"))?;
        let settings = match store.meta::<Settings>("settings") {
            Some(saved) => {
                let (settings, changed) = saved.migrate();
                if changed {
                    let _ = store.set_meta("settings", &settings);
                }
                settings
            }
            None => Settings::default(),
        };
        let me = store.meta::<String>("me").unwrap_or_default();
        let chats = store.chats().into_iter().map(|c| (c.id.clone(), c)).collect();
        Ok(Arc::new(App {
            store,
            media,
            root,
            demo,
            events,
            net: RwLock::new(Arc::new(NoNetwork)),
            session: Mutex::new(Session::Welcome),
            me: RwLock::new(me),
            chats: Mutex::new(chats),
            active: Mutex::new(None),
            focused: AtomicBool::new(true),
            settings: RwLock::new(settings),
            badge: AtomicU32::new(u32::MAX),
            history_asked: Mutex::new(HashMap::new()),
            sticker_jobs: Mutex::new(HashSet::new()),
            #[cfg(feature = "voice")]
            recorder: Mutex::new(None),
        }))
    }

    /// Installs the network backend and starts it, plus the background sweeper.
    pub async fn start(self: &Arc<Self>, net: Arc<dyn Network>) -> Result<()> {
        *self.net.write().unwrap_or_else(|e| e.into_inner()) = net.clone();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                let Some(app) = weak.upgrade() else { break };
                app.sweep_expired();
            }
        });
        net.start(self.clone()).await
    }

    pub fn net(&self) -> Arc<dyn Network> {
        self.net.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn emit(&self, ev: UiEvent) {
        let _ = self.events.send(ev);
    }

    pub fn toast(&self, tone: &str, text: impl Into<String>) {
        self.emit(UiEvent::Toast { tone: tone.into(), text: text.into() });
    }

    pub fn me(&self) -> Jid {
        self.me.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn session(&self) -> Session {
        lock(&self.session).clone()
    }

    /// Records who we are (after pairing) and makes sure our own contact exists.
    pub fn set_me(&self, jid: &str, push_name: Option<String>) {
        let jid = bare(jid);
        *self.me.write().unwrap_or_else(|e| e.into_inner()) = jid.clone();
        let _ = self.store.set_meta("me", &jid);
        let c = self.store.upsert_contact(&jid, |c| {
            c.is_me = true;
            if push_name.is_some() {
                c.push_name = push_name;
            }
        });
        self.emit(UiEvent::Me { contact: c });
    }

    pub fn set_session(&self, s: Session) {
        let mut cur = lock(&self.session);
        if *cur != s {
            *cur = s.clone();
            drop(cur);
            self.emit(UiEvent::Session { session: s });
        }
    }

    pub fn set_privacy(&self, p: Privacy) {
        let _ = self.store.set_meta("privacy", &p);
        self.emit(UiEvent::Privacy { privacy: p });
    }

    // ───────────────────────────── chats ─────────────────────────────

    pub fn chat(&self, id: &str) -> Option<Chat> {
        lock(&self.chats).get(id).cloned()
    }

    pub fn all_chats(&self) -> Vec<Chat> {
        lock(&self.chats).values().cloned().collect()
    }

    pub fn display_name(&self, jid: &str) -> String {
        if is_group(jid) {
            return self.store.group(jid).map(|g| g.subject).filter(|s| !s.is_empty()).unwrap_or_else(|| "Group".into());
        }
        self.store.contact(jid).map(|c| c.display()).unwrap_or_else(|| phone_of(jid))
    }

    fn save_chat(&self, chat: &Chat) {
        let _ = self.store.put_chat(chat);
        self.emit(UiEvent::Chat { chat: chat.clone() });
        self.refresh_badge();
    }

    /// Returns the chat, creating an empty one when it does not exist yet.
    pub fn ensure_chat(&self, id: &str) -> Chat {
        if let Some(c) = self.chat(id) {
            return c;
        }
        let mut chat = Chat { id: id.to_string(), name: self.display_name(id), ts: now_ms(), ..Default::default() };
        if is_group(id) {
            chat.kind = ChatKind::Group;
            if let Some(g) = self.store.group(id) {
                self.apply_group_to_chat(&g, &mut chat);
            }
        } else if let Some(c) = self.store.contact(id) {
            chat.avatar = c.avatar;
        }
        lock(&self.chats).insert(id.to_string(), chat.clone());
        self.save_chat(&chat);
        chat
    }

    /// Mutates a chat in place, persists it and notifies the UI.
    pub fn chat_mut(&self, id: &str, f: impl FnOnce(&mut Chat)) -> Option<Chat> {
        let chat = {
            let mut map = lock(&self.chats);
            let c = map.get_mut(id)?;
            f(c);
            c.clone()
        };
        self.save_chat(&chat);
        Some(chat)
    }

    pub fn remove_chat(&self, id: &str) {
        lock(&self.chats).remove(id);
        let _ = self.store.delete_chat(id);
        self.emit(UiEvent::ChatRemoved { id: id.to_string() });
        self.refresh_badge();
    }

    fn refresh_badge(&self) {
        let now = now_ms();
        let count = lock(&self.chats)
            .values()
            .filter(|c| !c.archived && (c.unread > 0 || c.marked_unread))
            .filter(|c| c.muted_until == 0 || (c.muted_until > 0 && c.muted_until < now))
            .count() as u32;
        if self.badge.swap(count, Ordering::Relaxed) != count {
            self.emit(UiEvent::Badge { count });
        }
    }

    fn is_viewing(&self, chat: &str) -> bool {
        self.focused.load(Ordering::Relaxed) && lock(&self.active).as_deref() == Some(chat)
    }

    fn apply_group_to_chat(&self, g: &Group, chat: &mut Chat) {
        let me = self.me();
        chat.kind = if g.is_announcement { ChatKind::Announcement } else { ChatKind::Group };
        chat.name = g.subject.clone();
        chat.community = g.parent.clone();
        chat.ephemeral = g.ephemeral;
        if g.avatar.is_some() {
            chat.avatar = g.avatar.clone();
        }
        let role = g.role_of(&me).unwrap_or(Role::Member);
        chat.read_only = !g.joined || (g.announce && role == Role::Member);
    }

    // ───────────────────────────── ingest: messages ─────────────────────────────

    /// Stores a message that arrived (or that we just composed) and updates
    /// the chat row. `fresh` is false for history. Returns `None` for duplicates.
    pub fn ingest_message(self: &Arc<Self>, m: Message, fresh: bool) -> Option<Message> {
        self.ingest(m, fresh, fresh)
    }

    /// A message that arrived while we were offline: it counts as unread, but
    /// raises no notification and does not animate in.
    pub fn ingest_offline(self: &Arc<Self>, m: Message) -> Option<Message> {
        self.ingest(m, true, false)
    }

    fn ingest(self: &Arc<Self>, mut m: Message, fresh: bool, live: bool) -> Option<Message> {
        self.ensure_chat(&m.chat);
        match self.store.insert_message(&mut m) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(e) => {
                log::error!("insert message failed: {e:#}");
                return None;
            }
        }
        let me = self.me();
        let viewing = self.is_viewing(&m.chat);
        let counts = fresh && !m.from_me && !matches!(m.content, Content::System { .. });
        let mentioned = m.mentions.contains(&me) || m.quote.as_ref().is_some_and(|q| q.sender == me);
        let chat = self.chat_mut(&m.chat, |c| {
            if c.last.as_ref().is_none_or(|l| l.ts <= m.ts) {
                // A chat with no messages yet sorts by its creation time; once it has one, by that.
                c.ts = if c.last.is_none() { m.ts } else { c.ts.max(m.ts) };
                c.last = Some(m.preview());
            }
            if counts && !viewing {
                c.unread += 1;
                if mentioned {
                    c.unread_mentions += 1;
                }
            }
            if m.from_me && fresh {
                c.unread = 0;
                c.unread_mentions = 0;
                c.marked_unread = false;
                c.draft = None;
            }
        });
        self.emit(UiEvent::Message { message: m.clone(), fresh: live });
        if counts {
            if viewing {
                self.send_read_receipts(&m.chat, vec![m.clone()]);
            } else if let (true, Some(chat)) = (live, chat) {
                self.notify(&chat, &m);
            }
        }
        Some(m)
    }

    /// Bulk-stores history. Chats are updated once at the end.
    pub fn ingest_history(self: &Arc<Self>, mut msgs: Vec<Message>) -> usize {
        if msgs.is_empty() {
            return 0;
        }
        for m in &msgs {
            self.ensure_chat_quiet(&m.chat);
        }
        let added = self.store.insert_messages(&mut msgs).unwrap_or(0);
        let mut newest: HashMap<&str, &Message> = HashMap::new();
        for m in &msgs {
            let e = newest.entry(m.chat.as_str()).or_insert(m);
            if m.ts > e.ts {
                *e = m;
            }
        }
        for (chat, m) in newest {
            self.chat_mut(chat, |c| {
                if c.last.as_ref().is_none_or(|l| l.ts <= m.ts) {
                    c.ts = if c.last.is_none() { m.ts } else { c.ts.max(m.ts) };
                    c.last = Some(m.preview());
                }
            });
            self.emit(UiEvent::History { chat: chat.to_string() });
        }
        added
    }

    fn ensure_chat_quiet(&self, id: &str) {
        if lock(&self.chats).contains_key(id) {
            return;
        }
        self.ensure_chat(id);
    }

    /// Muted and archived chats never notify, not even for a mention or a reply.
    fn notify(&self, chat: &Chat, m: &Message) {
        let s = self.settings();
        let now = now_ms();
        let muted = chat.muted_until == -1 || chat.muted_until > now;
        if !s.notifications || chat.archived || muted {
            return;
        }
        // Names come from the contact book, not `chat.name`, which is fixed when the
        // chat is created and is often still a bare phone number.
        let title = self.display_name(&chat.id);
        let body = if chat.kind == ChatKind::Direct {
            m.content.summary()
        } else {
            format!("{}: {}", self.display_name(&m.sender), m.content.summary())
        };
        let body = if s.notification_preview { truncate(&body, 180) } else { "New message".into() };
        self.emit(UiEvent::Notify { chat: chat.id.clone(), id: m.id.clone(), title, body, avatar: chat.avatar.clone() });
    }

    /// Changes a stored message and tells the UI. Also refreshes the chat
    /// preview when the message is the latest one.
    pub fn message_mut(&self, chat: &str, id: &str, f: impl FnOnce(&mut Message)) -> Option<Message> {
        let m = self.store.update_message(chat, id, f)?;
        self.emit(UiEvent::Message { message: m.clone(), fresh: false });
        if self.chat(chat).and_then(|c| c.last).is_some_and(|l| l.id == id) {
            self.chat_mut(chat, |c| c.last = Some(m.preview()));
        }
        Some(m)
    }

    /// A delivery/read/played update for one of our messages. `by` is the
    /// user it concerns (needed for groups, where every member reports).
    pub fn ingest_status(&self, chat: &str, id: &str, status: Status, by: Option<&str>, ts: i64) {
        let Some(m) = self.store.message(chat, id) else { return };
        if !m.from_me {
            return;
        }
        let mut target = status;
        if let Some(user) = by {
            let _ = self.store.put_receipt(chat, id, user, status, ts);
            if is_group(chat) && matches!(status, Status::Delivered | Status::Read | Status::Played) {
                let others = self
                    .store
                    .group(chat)
                    .map(|g| g.participants.len().saturating_sub(1))
                    .unwrap_or(1)
                    .max(1);
                let r = self.store.receipts(chat, id);
                let read = r.iter().filter(|x| x.read > 0 || x.played > 0).count();
                let delivered = r.iter().filter(|x| x.delivered > 0 || x.read > 0 || x.played > 0).count();
                target = if status == Status::Played && r.iter().filter(|x| x.played > 0).count() >= others {
                    Status::Played
                } else if read >= others {
                    Status::Read
                } else if delivered >= others {
                    Status::Delivered
                } else {
                    Status::Sent
                };
            }
        }
        let upgrade = status == Status::Failed || target > m.status;
        if upgrade {
            let new = if status == Status::Failed { Status::Failed } else { target };
            self.message_mut(chat, id, |m| m.status = new);
        } else if by.is_some() {
            // Receipt details changed even though the tick did not.
            self.emit(UiEvent::Message { message: m, fresh: false });
        }
    }

    pub fn ingest_reaction(&self, chat: &str, id: &str, sender: &str, emoji: &str, ts: i64) {
        self.message_mut(chat, id, |m| {
            m.reactions.retain(|r| r.sender != sender);
            if !emoji.is_empty() {
                m.reactions.push(Reaction { emoji: emoji.to_string(), sender: sender.to_string(), ts });
            }
        });
    }

    pub fn ingest_edit(&self, chat: &str, id: &str, text: &str, ts: i64) {
        self.message_mut(chat, id, |m| {
            m.content.set_text(text.to_string());
            m.edited = ts;
        });
    }

    pub fn ingest_revoke(&self, chat: &str, id: &str) {
        self.message_mut(chat, id, |m| {
            m.content = Content::Deleted;
            m.reactions.clear();
            m.quote = None;
            m.starred = false;
            m.pinned_until = 0;
        });
    }

    pub fn ingest_poll_vote(&self, chat: &str, id: &str, voter: &str, chosen: &[String]) {
        self.message_mut(chat, id, |m| {
            if let Content::Poll { options, .. } = &mut m.content {
                for o in options.iter_mut() {
                    o.voters.retain(|v| v != voter);
                    if chosen.contains(&o.name) {
                        o.voters.push(voter.to_string());
                    }
                }
            }
        });
    }

    pub fn ingest_rsvp(&self, chat: &str, id: &str, user: &str, response: &str) {
        self.message_mut(chat, id, |m| {
            if let Content::Event { responses, .. } = &mut m.content {
                for list in [&mut responses.going, &mut responses.maybe, &mut responses.declined] {
                    list.retain(|u| u != user);
                }
                match response {
                    "going" => responses.going.push(user.to_string()),
                    "maybe" => responses.maybe.push(user.to_string()),
                    "declined" => responses.declined.push(user.to_string()),
                    _ => {}
                }
            }
        });
    }

    /// Pins or unpins (`until == 0`) a message, with the system line WhatsApp shows.
    pub fn ingest_pin(self: &Arc<Self>, chat: &str, id: &str, until: i64, by: &str) {
        if self.message_mut(chat, id, |m| m.pinned_until = until).is_some() && until > 0 {
            self.system_message(chat, "pin", Some(by), &[], id);
        }
    }

    pub fn ingest_star(&self, chat: &str, id: &str, on: bool) {
        self.message_mut(chat, id, |m| m.starred = on);
    }

    pub fn ingest_ephemeral(self: &Arc<Self>, chat: &str, secs: u32, by: &str) {
        let changed = self.chat(chat).is_some_and(|c| c.ephemeral != secs);
        self.chat_mut(chat, |c| c.ephemeral = secs);
        self.store.update_group(chat, |g| g.ephemeral = secs);
        if changed {
            self.system_message(chat, "ephemeral", Some(by), &[], &secs.to_string());
        }
    }

    pub fn ingest_deleted_for_me(&self, chat: &str, id: &str) {
        let _ = self.store.delete_message(chat, id);
        self.emit(UiEvent::MessageRemoved { chat: chat.to_string(), id: id.to_string() });
        self.fix_preview(chat);
    }

    fn fix_preview(&self, chat: &str) {
        let last = self.store.last_message(chat).map(|m| m.preview());
        self.chat_mut(chat, |c| c.last = last);
    }

    /// Adds a centred system line ("Ana added Marco", "You pinned a message", …).
    pub fn system_message(self: &Arc<Self>, chat: &str, event: &str, actor: Option<&str>, targets: &[Jid], text: &str) {
        let m = Message {
            id: format!("SYS{}", media::random_hex(8).to_uppercase()),
            chat: chat.to_string(),
            seq: 0,
            sender: actor.unwrap_or_default().to_string(),
            from_me: actor.is_some_and(|a| a == self.me()),
            ts: now_ms(),
            content: Content::System {
                event: event.to_string(),
                actor: actor.map(String::from),
                targets: targets.to_vec(),
                text: text.to_string(),
            },
            status: Status::Read,
            quote: None,
            reactions: vec![],
            mentions: vec![],
            starred: false,
            pinned_until: 0,
            edited: 0,
            forwarded: 0,
            view_once: false,
            expires: 0,
            album: None,
        };
        self.ingest_message(m, true);
    }

    // ───────────────────────────── ingest: everything else ─────────────────────────────

    pub fn ingest_typing(&self, chat: &str, user: &str, state: &str) {
        self.emit(UiEvent::Typing { chat: chat.to_string(), user: user.to_string(), state: state.to_string() });
    }

    pub fn ingest_presence(&self, jid: &str, online: bool, last_seen: i64) {
        self.emit(UiEvent::Presence { jid: jid.to_string(), online, last_seen });
    }

    /// Creates or updates a contact; keeps the matching direct chat's name and avatar in step.
    pub fn ingest_contact(&self, id: &str, f: impl FnOnce(&mut Contact)) -> Contact {
        let c = self.store.upsert_contact(id, f);
        self.emit(UiEvent::Contact { contact: c.clone() });
        if c.is_me {
            self.emit(UiEvent::Me { contact: c.clone() });
        }
        if self.chat(id).is_some_and(|ch| ch.name != c.display() || ch.avatar != c.avatar) {
            self.chat_mut(id, |ch| {
                ch.name = c.display();
                ch.avatar = c.avatar.clone();
            });
        }
        c
    }

    /// Stores group metadata. Joined, non-community groups get a chat row.
    pub fn ingest_group(&self, g: Group) {
        let _ = self.store.put_group(&g);
        self.emit(UiEvent::Group { group: g.clone() });
        if g.is_community {
            return;
        }
        if self.chat(&g.id).is_some() {
            self.chat_mut(&g.id, |c| self.apply_group_to_chat(&g, c));
        } else if g.joined {
            self.ensure_chat(&g.id);
        }
    }

    pub fn group_mut(&self, id: &str, f: impl FnOnce(&mut Group)) -> Option<Group> {
        let g = self.store.update_group(id, f)?;
        self.ingest_group(g.clone());
        Some(g)
    }

    pub fn remove_group(&self, id: &str) {
        let _ = self.store.delete_group(id);
        self.emit(UiEvent::GroupRemoved { id: id.to_string() });
    }

    pub fn ingest_call(self: &Arc<Self>, id: &str, from: &str, video: bool, ringing: bool, missed: bool) {
        self.emit(UiEvent::Call { id: id.to_string(), from: from.to_string(), video, ringing });
        if !ringing && missed {
            let m = Message {
                id: format!("CALL{id}"),
                chat: bare(from),
                seq: 0,
                sender: bare(from),
                from_me: false,
                ts: now_ms(),
                content: Content::Call { video, missed: true, duration: 0 },
                status: Status::Read,
                quote: None,
                reactions: vec![],
                mentions: vec![],
                starred: false,
                pinned_until: 0,
                edited: 0,
                forwarded: 0,
                view_once: false,
                expires: 0,
                album: None,
            };
            self.ingest_message(m, true);
        }
    }

    /// Marks downloaded media as available (or failed) on a message.
    pub fn set_media(&self, chat: &str, id: &str, token: Option<String>) {
        self.message_mut(chat, id, |m| {
            if let Some(media) = m.content.media_mut() {
                match &token {
                    Some(t) => {
                        media.token = Some(t.clone());
                        media.state = MediaState::Ready;
                    }
                    None => media.state = MediaState::Failed,
                }
            }
        });
    }

    fn sweep_expired(&self) {
        for (chat, id) in self.store.expired(now_ms()) {
            self.ingest_deleted_for_me(&chat, &id);
        }
    }

    // ───────────────────────────── outgoing helpers ─────────────────────────────

    fn send_read_receipts(self: &Arc<Self>, chat: &str, msgs: Vec<Message>) {
        if msgs.is_empty() {
            return;
        }
        let net = self.net();
        let chat = chat.to_string();
        tokio::spawn(async move {
            if let Err(e) = net.exec(NetOp::MarkRead { chat, msgs }).await {
                log::warn!("read receipt failed: {e:#}");
            }
        });
    }

    fn mark_read(self: &Arc<Self>, chat: &str) {
        let Some(c) = self.chat(chat) else { return };
        if c.unread == 0 && !c.marked_unread {
            return;
        }
        let n = (c.unread as usize).min(300);
        self.chat_mut(chat, |c| {
            c.unread = 0;
            c.unread_mentions = 0;
            c.marked_unread = false;
        });
        if n > 0 {
            let mut incoming: Vec<Message> =
                self.store.messages(chat, None, n + 40).into_iter().filter(|m| !m.from_me && !m.is_system()).collect();
            let skip = incoming.len().saturating_sub(n);
            self.send_read_receipts(chat, incoming.split_off(skip));
        }
        if c.marked_unread {
            self.fire(NetOp::Chat { chat: chat.to_string(), action: ChatAction::MarkRead(true), last: self.last_boxed(chat) });
        }
    }

    fn last_boxed(&self, chat: &str) -> Option<Box<Message>> {
        self.store.last_message(chat).map(Box::new)
    }

    /// Runs a network op in the background; failures surface as a toast.
    fn fire(self: &Arc<Self>, op: NetOp) {
        let app = self.clone();
        tokio::spawn(async move {
            if let Err(e) = app.net().exec(op).await {
                log::warn!("network op failed: {e:#}");
                app.toast("error", format!("{e}"));
            }
        });
    }

    fn base_message(&self, chat: &Chat, content: Content) -> Message {
        let now = now_ms();
        Message {
            id: new_message_id(),
            chat: chat.id.clone(),
            seq: 0,
            sender: self.me(),
            from_me: true,
            ts: now,
            content,
            status: Status::Pending,
            quote: None,
            reactions: vec![],
            mentions: vec![],
            starred: false,
            pinned_until: 0,
            edited: 0,
            forwarded: 0,
            view_once: false,
            expires: if chat.ephemeral > 0 { now + chat.ephemeral as i64 * 1000 } else { 0 },
            album: None,
        }
    }

    /// Stores an outgoing message and hands it to the network.
    fn dispatch(self: &Arc<Self>, m: Message) -> Option<Message> {
        let m = self.ingest_message(m, true)?;
        let app = self.clone();
        let sent = m.clone();
        tokio::spawn(async move {
            let (chat, id) = (sent.chat.clone(), sent.id.clone());
            match app.net().exec(NetOp::Send { msg: Box::new(sent) }).await {
                Ok(_) => {
                    if app.store.message(&chat, &id).is_some_and(|m| m.status == Status::Pending) {
                        app.message_mut(&chat, &id, |m| m.status = Status::Sent);
                    }
                }
                Err(e) => {
                    log::warn!("send failed: {e:#}");
                    app.message_mut(&chat, &id, |m| m.status = Status::Failed);
                    app.toast("error", format!("Message not sent: {e}"));
                }
            }
        });
        Some(m)
    }

    fn build_media(&self, file: &Attachment, kind: &str, caption: String) -> Result<Content> {
        let (token, size) = self.media.import(file)?;
        let path = self.media.path(&token).ok_or_else(|| anyhow!("bad media path"))?;
        let mime = if file.mime.is_empty() { media::mime_for(&file.name).to_string() } else { file.mime.clone() };
        let mut m = Media { token: Some(token.clone()), mime, size, state: MediaState::Ready };
        Ok(match kind {
            "image" => {
                let (width, height) = media::dimensions(&path).unwrap_or((file.width.max(1), file.height.max(1)));
                Content::Image { media: m, caption, width, height, thumb: media::thumb_b64(&path, 64) }
            }
            "video" | "gif" => Content::Video {
                media: m,
                caption,
                width: file.width.max(1),
                height: file.height.max(1),
                duration: file.duration,
                gif: kind == "gif",
                thumb: file.thumb.as_ref().and_then(|t| {
                    use base64::Engine;
                    let raw = t.split(";base64,").last().unwrap_or(t);
                    base64::engine::general_purpose::STANDARD.decode(raw).ok().and_then(|b| media::thumb_from_bytes(&b, 96))
                }),
            },
            "audio" | "voice" => Content::Audio { media: m, duration: file.duration, voice: kind == "voice", waveform: vec![] },
            "sticker" => {
                let bytes = std::fs::read(&path)?;
                let (bytes, animated) = if file.mime == "image/webp" {
                    let animated = media::is_animated_webp(&bytes);
                    (bytes, animated)
                } else {
                    (media::to_sticker(&path)?, false)
                };
                let _ = std::fs::remove_file(&path);
                let tok = self.media.write("s", "webp", &bytes)?;
                m.token = Some(tok.clone());
                m.mime = "image/webp".into();
                m.size = bytes.len() as u64;
                self.ingest_sticker(&file_hash(&bytes), |s| {
                    s.token = Some(tok);
                    s.animated = animated;
                    s.used = now_ms();
                });
                Content::Sticker { media: m, width: 512, height: 512, animated }
            }
            _ => Content::Document {
                media: m,
                name: if file.name.is_empty() { "Document".into() } else { file.name.clone() },
                caption,
                pages: 0,
                thumb: if file.mime.starts_with("image/") { media::thumb_b64(&path, 96) } else { None },
            },
        })
    }

    fn build_content(&self, out: Outgoing) -> Result<(Content, bool, Option<String>)> {
        Ok(match out {
            Outgoing::Text { text, preview } => {
                if text.trim().is_empty() {
                    bail!("Message is empty");
                }
                (Content::Text { text, preview }, false, None)
            }
            Outgoing::Media { file, kind, caption, view_once, album } => {
                (self.build_media(&file, &kind, caption)?, view_once, album)
            }
            Outgoing::Sticker { id } => {
                let row = self.store.sticker(&id).or_else(|| self.store.sticker_by_token(&id)).ok_or_else(|| anyhow!("Sticker is missing"))?;
                let token = row.token.clone().ok_or_else(|| anyhow!("This sticker is still downloading"))?;
                let path = self.media.path(&token).filter(|p| p.exists()).ok_or_else(|| anyhow!("Sticker is missing"))?;
                let bytes = std::fs::read(&path)?;
                let animated = media::is_animated_webp(&bytes);
                // Stickers saved by older versions were keyed by file token; move them to their hash.
                let hash = file_hash(&bytes);
                if row.id != hash {
                    let _ = self.store.remove_sticker(&row.id);
                }
                self.ingest_sticker(&hash, |s| {
                    let used = now_ms();
                    *s = Sticker { id: hash.clone(), used, animated, ..row.clone() };
                });
                let m = Media {
                    token: Some(token),
                    mime: "image/webp".into(),
                    size: bytes.len() as u64,
                    state: MediaState::Ready,
                };
                (Content::Sticker { media: m, width: 512, height: 512, animated }, false, None)
            }
            Outgoing::Location { lat, lng, name, address } => {
                (Content::Location { lat, lng, name, address, live: false, thumb: None }, false, None)
            }
            Outgoing::Contacts { jids } => {
                let cards = jids
                    .iter()
                    .map(|j| {
                        let name = self.display_name(j);
                        let phone = phone_of(j);
                        let wa = phone.trim_start_matches('+').to_string();
                        ContactCard {
                            vcard: format!(
                                "BEGIN:VCARD\nVERSION:3.0\nFN:{name}\nTEL;type=CELL;waid={wa}:{phone}\nEND:VCARD"
                            ),
                            name,
                            phone: Some(phone),
                            jid: Some(j.clone()),
                        }
                    })
                    .collect::<Vec<_>>();
                if cards.is_empty() {
                    bail!("Pick at least one contact");
                }
                (Content::Contacts { cards }, false, None)
            }
            Outgoing::Poll { question, options, selectable } => {
                let options: Vec<PollOption> = options
                    .into_iter()
                    .map(|o| o.trim().to_string())
                    .filter(|o| !o.is_empty())
                    .map(|name| PollOption { name, voters: vec![] })
                    .collect();
                if question.trim().is_empty() || options.len() < 2 {
                    bail!("A poll needs a question and at least two options");
                }
                (Content::Poll { question, options, selectable }, false, None)
            }
            Outgoing::Event { name, description, location, start, end } => {
                if name.trim().is_empty() {
                    bail!("The event needs a name");
                }
                let responses = EventResponses { going: vec![self.me()], ..Default::default() };
                (Content::Event { name, description, location, start, end, canceled: false, responses }, false, None)
            }
        })
    }

    fn send(
        self: &Arc<Self>,
        chat: &str,
        content: Outgoing,
        reply_to: Option<String>,
        mentions: Vec<Jid>,
    ) -> Result<Value> {
        let chat_row = self.ensure_chat(chat);
        if chat_row.read_only {
            bail!("You can't send messages to this chat");
        }
        let (content, view_once, album) = self.build_content(content)?;
        let mut m = self.base_message(&chat_row, content);
        m.quote = reply_to.and_then(|id| self.store.message(chat, &id)).map(|q| q.quote());
        m.mentions = mentions;
        m.view_once = view_once;
        m.album = album;
        val(self.dispatch(m))
    }

    // ───────────────────────────── requests ─────────────────────────────

    pub async fn handle(self: &Arc<Self>, req: Request) -> Result<Value> {
        match req {
            Request::Init => {
                let me = self.me();
                val(json!({
                    "session": self.session(),
                    "me": if me.is_empty() { None } else { self.store.contact(&me) },
                    "settings": self.settings(),
                    "privacy": self.store.meta::<Privacy>("privacy").unwrap_or_default(),
                    "chats": self.all_chats(),
                    "contacts": self.store.contacts(),
                    "groups": self.store.groups(),
                    "demo": self.demo,
                    "platform": std::env::consts::OS,
                    "version": env!("CARGO_PKG_VERSION"),
                }))
            }
            Request::Pair { phone } => self.net().exec(NetOp::Pair { phone }).await,
            Request::Logout => self.net().exec(NetOp::Logout).await,
            Request::DemoStart | Request::DemoExit => bail!("handled by the host"),
            Request::DemoTrigger { what } => self.net().exec(NetOp::DemoTrigger { what }).await,
            Request::Focus { on } => {
                self.focused.store(on, Ordering::Relaxed);
                if on {
                    let active = lock(&self.active).clone();
                    if let Some(chat) = active {
                        self.mark_read(&chat);
                    }
                }
                self.fire_quiet(NetOp::Available { on });
                Ok(Value::Null)
            }

            // ── chats ──
            Request::OpenChat { chat } => {
                let row = self.ensure_chat(&chat);
                *lock(&self.active) = Some(chat.clone());
                let unread = row.unread;
                if self.focused.load(Ordering::Relaxed) {
                    self.mark_read(&chat);
                }
                if row.kind == ChatKind::Direct {
                    self.fire_quiet(NetOp::Subscribe { jid: chat.clone() });
                }
                let limit = PAGE.max(unread as usize + 20).min(400);
                let messages = self.store.messages(&chat, None, limit);
                if messages.len() < 20 {
                    self.ask_history(&chat, messages.first().cloned());
                }
                val(json!({
                    "messages": messages,
                    "pinned": self.store.pinned(&chat, now_ms()),
                    "unread": unread,
                    "group": self.store.group(&chat),
                }))
            }
            Request::CloseChat => {
                *lock(&self.active) = None;
                Ok(Value::Null)
            }
            Request::Messages { chat, before, after, limit } => {
                let limit = limit.unwrap_or(PAGE).min(400);
                let list = match after {
                    Some(c) => self.store.messages_after(&chat, c, limit),
                    None => {
                        let list = self.store.messages(&chat, before, limit);
                        if list.len() < limit {
                            let oldest = list.first().cloned().or_else(|| self.store.messages(&chat, None, 1).pop());
                            self.ask_history(&chat, oldest);
                        }
                        list
                    }
                };
                val(list)
            }
            Request::MessagesAround { chat, id } => val(self.store.messages_around(&chat, &id, 40)),
            Request::MarkRead { chat } => {
                self.mark_read(&chat);
                Ok(Value::Null)
            }
            Request::MarkUnread { chat } => {
                self.chat_mut(&chat, |c| c.marked_unread = true);
                self.fire(NetOp::Chat { chat: chat.clone(), action: ChatAction::MarkRead(false), last: self.last_boxed(&chat) });
                Ok(Value::Null)
            }
            Request::Archive { chat, on } => {
                self.chat_mut(&chat, |c| {
                    c.archived = on;
                    if on {
                        c.pinned = 0;
                    }
                });
                self.fire(NetOp::Chat { chat: chat.clone(), action: ChatAction::Archive(on), last: self.last_boxed(&chat) });
                Ok(Value::Null)
            }
            Request::PinChat { chat, on } => {
                if on && lock(&self.chats).values().filter(|c| c.pinned > 0 && !c.archived).count() >= 3 {
                    bail!("You can pin up to 3 chats");
                }
                self.chat_mut(&chat, |c| c.pinned = if on { now_ms() } else { 0 });
                self.fire(NetOp::Chat { chat, action: ChatAction::Pin(on), last: None });
                Ok(Value::Null)
            }
            Request::Mute { chat, until } => {
                self.chat_mut(&chat, |c| c.muted_until = until);
                self.fire(NetOp::Chat { chat, action: ChatAction::Mute(until), last: None });
                Ok(Value::Null)
            }
            Request::Favorite { chat, on } => {
                self.chat_mut(&chat, |c| c.favorite = on);
                Ok(Value::Null)
            }
            Request::DeleteChat { chat } => {
                let last = self.last_boxed(&chat);
                if lock(&self.active).as_deref() == Some(chat.as_str()) {
                    *lock(&self.active) = None;
                }
                self.remove_chat(&chat);
                self.fire(NetOp::Chat { chat, action: ChatAction::Delete, last });
                Ok(Value::Null)
            }
            Request::ClearChat { chat } => {
                let last = self.last_boxed(&chat);
                self.store.clear_chat(&chat)?;
                self.chat_mut(&chat, |c| {
                    c.last = None;
                    c.unread = 0;
                    c.unread_mentions = 0;
                });
                self.emit(UiEvent::History { chat: chat.clone() });
                self.fire(NetOp::Chat { chat, action: ChatAction::Clear, last });
                Ok(Value::Null)
            }
            Request::Draft { chat, text } => {
                let draft = if text.trim().is_empty() { None } else { Some(text) };
                if self.chat(&chat).is_some_and(|c| c.draft != draft) {
                    self.chat_mut(&chat, |c| c.draft = draft);
                }
                Ok(Value::Null)
            }
            Request::Ephemeral { chat, secs } => {
                self.net().exec(NetOp::Ephemeral { chat: chat.clone(), secs }).await?;
                let me = self.me();
                self.ingest_ephemeral(&chat, secs, &me);
                Ok(Value::Null)
            }
            Request::Typing { chat, state } => {
                self.fire_quiet(NetOp::Typing { chat, state });
                Ok(Value::Null)
            }
            Request::Subscribe { jid } => {
                self.fire_quiet(NetOp::Subscribe { jid });
                Ok(Value::Null)
            }
            Request::StartChat { jid } => val(self.ensure_chat(&bare(&jid))),
            Request::CheckNumber { phone } => self.net().exec(NetOp::CheckNumber { phone }).await,

            // ── messages ──
            Request::Send { chat, content, reply_to, mentions } => self.send(&chat, content, reply_to, mentions),
            Request::Retry { chat, id } => {
                let m = self.message_mut(&chat, &id, |m| m.status = Status::Pending).ok_or_else(|| anyhow!("Message not found"))?;
                let app = self.clone();
                tokio::spawn(async move {
                    let ok = app.net().exec(NetOp::Send { msg: Box::new(m) }).await.is_ok();
                    app.message_mut(&chat, &id, |m| {
                        if m.status == Status::Pending {
                            m.status = if ok { Status::Sent } else { Status::Failed };
                        }
                    });
                });
                Ok(Value::Null)
            }
            Request::React { chat, id, emoji } => {
                let me = self.me();
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                // Reacting again with the same emoji removes it.
                let emoji = if target.reactions.iter().any(|r| r.sender == me && r.emoji == emoji) { String::new() } else { emoji };
                self.ingest_reaction(&chat, &id, &me, &emoji, now_ms());
                self.fire(NetOp::React { target: Box::new(target), emoji });
                Ok(Value::Null)
            }
            Request::Edit { chat, id, text } => {
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                if !target.from_me {
                    bail!("You can only edit your own messages");
                }
                if text.trim().is_empty() {
                    bail!("Message is empty");
                }
                self.ingest_edit(&chat, &id, &text, now_ms());
                self.fire(NetOp::Edit { target: Box::new(target), text });
                Ok(Value::Null)
            }
            Request::Delete { chat, ids, everyone } => {
                let mut local = vec![];
                for id in &ids {
                    let Some(m) = self.store.message(&chat, id) else { continue };
                    if everyone {
                        self.ingest_revoke(&chat, id);
                        self.fire(NetOp::Revoke { target: Box::new(m) });
                    } else {
                        self.ingest_deleted_for_me(&chat, id);
                        local.push(m);
                    }
                }
                if !local.is_empty() {
                    self.fire(NetOp::DeleteForMe { targets: local });
                }
                Ok(Value::Null)
            }
            Request::Star { chat, ids, on } => {
                let targets: Vec<Message> = ids.iter().filter_map(|id| self.message_mut(&chat, id, |m| m.starred = on)).collect();
                self.fire(NetOp::Star { targets, on });
                Ok(Value::Null)
            }
            Request::PinMessage { chat, id, secs } => {
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let until = if secs == 0 { 0 } else { now_ms() + secs as i64 * 1000 };
                if until > 0 && self.store.pinned(&chat, now_ms()).len() >= 3 {
                    bail!("You can pin up to 3 messages in a chat");
                }
                let me = self.me();
                self.ingest_pin(&chat, &id, until, &me);
                self.fire(NetOp::Pin { target: Box::new(target), secs });
                Ok(Value::Null)
            }
            Request::Forward { chat, ids, to } => {
                let mut sources: Vec<Message> = ids.iter().filter_map(|id| self.store.message(&chat, id)).collect();
                sources.sort_by_key(|m| (m.ts, m.seq));
                for dest in &to {
                    let row = self.ensure_chat(dest);
                    if row.read_only {
                        continue;
                    }
                    for src in &sources {
                        if matches!(src.content, Content::System { .. } | Content::Deleted | Content::Call { .. }) {
                            continue;
                        }
                        let mut content = src.content.clone();
                        if let Content::Poll { options, .. } = &mut content {
                            options.iter_mut().for_each(|o| o.voters.clear());
                        }
                        let mut m = self.base_message(&row, content);
                        m.forwarded = if src.from_me && src.forwarded == 0 { 1 } else { src.forwarded + 1 };
                        if let Some(media) = src.content.media()
                            && media.state != MediaState::Ready {
                                self.toast("error", "Download the media before forwarding it");
                                continue;
                            }
                        self.dispatch(m);
                    }
                }
                Ok(Value::Null)
            }
            Request::Vote { chat, id, options } => {
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Poll not found"))?;
                let me = self.me();
                self.ingest_poll_vote(&chat, &id, &me, &options);
                self.fire(NetOp::Vote { target: Box::new(target), options });
                Ok(Value::Null)
            }
            Request::Rsvp { chat, id, response } => {
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Event not found"))?;
                let me = self.me();
                self.ingest_rsvp(&chat, &id, &me, &response);
                self.fire(NetOp::Rsvp { target: Box::new(target), response });
                Ok(Value::Null)
            }
            Request::MessageInfo { chat, id } => val(json!({
                "message": self.store.message(&chat, &id),
                "receipts": self.store.receipts(&chat, &id),
            })),
            Request::Download { chat, id } => {
                let target = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let Some(media) = target.content.media() else { bail!("Nothing to download") };
                if matches!(media.state, MediaState::Ready | MediaState::Downloading) {
                    return Ok(Value::Null);
                }
                self.message_mut(&chat, &id, |m| {
                    if let Some(media) = m.content.media_mut() {
                        media.state = MediaState::Downloading;
                    }
                });
                let app = self.clone();
                tokio::spawn(async move {
                    let res = app.net().exec(NetOp::Download { target: Box::new(target) }).await;
                    let token = match res {
                        Ok(v) => v.get("token").and_then(|t| t.as_str()).map(String::from),
                        Err(e) => {
                            log::warn!("download failed: {e:#}");
                            None
                        }
                    };
                    app.set_media(&chat, &id, token);
                });
                Ok(Value::Null)
            }
            Request::Played { chat, id } => {
                if let Some(m) = self.store.message(&chat, &id)
                    && !m.from_me {
                        self.fire_quiet(NetOp::MarkPlayed { target: Box::new(m) });
                    }
                Ok(Value::Null)
            }
            Request::SaveMedia { chat, id, to } => {
                let m = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let token = m.content.media().and_then(|x| x.token.clone()).ok_or_else(|| anyhow!("Download the file first"))?;
                let src = self.media.path(&token).ok_or_else(|| anyhow!("File is missing"))?;
                std::fs::copy(src, &to).with_context(|| format!("save to {to}"))?;
                Ok(Value::Null)
            }

            // ── lookups ──
            Request::Starred { chat } => val(self.store.starred(chat.as_deref(), 300)),
            Request::Pinned { chat } => val(self.store.pinned(&chat, now_ms())),
            Request::Gallery { chat, tab, before } => val(match tab.as_str() {
                "docs" => self.store.gallery(&chat, &["document"], before, 60),
                "audio" => self.store.gallery(&chat, &["audio", "voice"], before, 60),
                "links" => self.store.links(&chat, 120),
                _ => self.store.gallery(&chat, &["image", "video", "gif"], before, 90),
            }),
            Request::Search { query, chat } => val(self.store.search(&query, chat.as_deref(), 80)),
            Request::LinkPreview { url } => {
                if !self.settings().link_previews {
                    return Ok(Value::Null);
                }
                let p = tokio::task::spawn_blocking(move || crate::link::fetch(&url)).await.ok().flatten();
                val(p)
            }

            // ── people ──
            Request::SaveContact { jid, name } => {
                let name = name.trim().to_string();
                self.ingest_contact(&jid, |c| c.name = if name.is_empty() { None } else { Some(name.clone()) });
                self.fire(NetOp::SaveContact { jid, name });
                Ok(Value::Null)
            }
            Request::Block { jid, on } => {
                self.net().exec(NetOp::Block { jid: jid.clone(), on }).await?;
                self.ingest_contact(&jid, |c| c.blocked = on);
                Ok(Value::Null)
            }
            Request::Avatar { jid } => {
                self.fire_quiet(NetOp::Avatar { jid });
                Ok(Value::Null)
            }
            Request::Group { op } => self.group_op(op).await,
            Request::Community { op } => self.community_op(op).await,
            Request::Profile { op } => {
                let me = self.me();
                match &op {
                    ProfileOp::Name { text } => {
                        let t = text.trim().to_string();
                        if t.is_empty() {
                            bail!("Your name can't be empty");
                        }
                        self.net().exec(NetOp::Profile(op.clone())).await?;
                        self.ingest_contact(&me, |c| c.push_name = Some(t));
                    }
                    ProfileOp::About { text } => {
                        self.net().exec(NetOp::Profile(op.clone())).await?;
                        let t = text.clone();
                        self.ingest_contact(&me, |c| c.about = Some(t));
                    }
                    ProfileOp::Photo { file } => {
                        let token = self.import_avatar(file.as_ref())?;
                        self.net().exec(NetOp::Profile(ProfileOp::Photo { file: self.avatar_attachment(&token) })).await?;
                        self.ingest_contact(&me, |c| c.avatar = token);
                    }
                }
                Ok(Value::Null)
            }

            // ── stickers ──
            Request::Stickers => val(self.sticker_list()),
            Request::StickerFavorite { id, on } => {
                let s = self.store.update_sticker(&id, |s| s.favorite = on)?;
                self.emit(UiEvent::Stickers);
                self.fire_quiet(NetOp::StickerFavorite { sticker: Box::new(s), on });
                val(self.store.stickers())
            }
            Request::StickerForget { id } => {
                if let Some(s) = self.store.sticker(&id) {
                    if s.used > 0 {
                        self.fire_quiet(NetOp::StickerUnrecent { sticker: Box::new(s.clone()) });
                    }
                    if s.favorite {
                        self.store.update_sticker(&id, |x| x.used = 0)?;
                    } else {
                        self.store.remove_sticker(&id)?;
                    }
                    self.emit(UiEvent::Stickers);
                }
                val(self.store.stickers())
            }
            Request::StickerCreate { file } => {
                self.build_media(&file, "sticker", String::new())?;
                val(json!({ "stickers": self.store.stickers() }))
            }
            Request::StickerFromMessage { chat, id } => {
                let m = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let Content::Sticker { media, animated, .. } = &m.content else { bail!("That message isn't a sticker") };
                let token = media.token.clone().filter(|_| media.state == MediaState::Ready).ok_or_else(|| anyhow!("Download the sticker first"))?;
                let path = self.media.path(&token).ok_or_else(|| anyhow!("Sticker is missing"))?;
                let bytes = std::fs::read(&path)?;
                let info = self.net().exec(NetOp::StickerInfo { target: Box::new(m.clone()) }).await.unwrap_or(Value::Null);
                let sid = info.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| file_hash(&bytes));
                let remote: Option<RemoteSticker> = info.get("remote").and_then(|r| serde_json::from_value(r.clone()).ok());
                // Keep a copy: cleared chat media must not take the favourite with it.
                let own = match self.store.sticker(&sid).and_then(|s| s.token).filter(|t| self.media.path(t).is_some_and(|p| p.exists())) {
                    Some(t) => t,
                    None => self.media.write("s", "webp", &bytes)?,
                };
                let animated = *animated;
                let s = self.store.update_sticker(&sid, |s| {
                    s.favorite = true;
                    s.token = Some(own);
                    s.animated = animated;
                    if s.remote.is_none() {
                        s.remote = remote;
                    }
                })?;
                self.emit(UiEvent::Stickers);
                self.fire_quiet(NetOp::StickerFavorite { sticker: Box::new(s), on: true });
                val(self.store.stickers())
            }
            Request::StickerDetails { chat, id } => {
                let m = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let Content::Sticker { media, width, height, animated } = &m.content else { bail!("That message isn't a sticker") };
                let bytes = media.token.as_deref().and_then(|t| self.media.path(t)).and_then(|p| std::fs::read(p).ok());
                // What WhatsApp says about it (its library id, description, flags).
                let info = self.net().exec(NetOp::StickerInfo { target: Box::new(m.clone()) }).await.unwrap_or(Value::Null);
                let sid = info
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| bytes.as_deref().map(file_hash));
                let favorite = sid.as_deref().and_then(|s| self.store.sticker(s)).is_some_and(|s| s.favorite);
                let pack = bytes.as_deref().and_then(media::sticker_pack_info).unwrap_or_default();
                let text = |k: &str| info.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                let flag = |k: &str| info.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
                // WhatsApp's own emoji tags are a plain string; the file's are a list.
                let mut emojis = pack.emojis;
                if emojis.is_empty()
                    && let Some(e) = text("emojis")
                {
                    emojis = e.split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect();
                }
                val(json!({
                    "id": sid,
                    "favorite": favorite,
                    "animated": animated,
                    "width": width,
                    "height": height,
                    "size": bytes.as_ref().map(|b| b.len()),
                    "label": text("label"),
                    "emojis": emojis,
                    "pack": pack.pack,
                    "publisher": pack.publisher,
                    "ai": flag("ai"),
                    "avatar": flag("avatar"),
                }))
            }
            Request::StickerSave { chat, id, to } => {
                let m = self.store.message(&chat, &id).ok_or_else(|| anyhow!("Message not found"))?;
                let token = m.content.media().and_then(|x| x.token.clone()).ok_or_else(|| anyhow!("Download the sticker first"))?;
                let src = self.media.path(&token).ok_or_else(|| anyhow!("Sticker is missing"))?;
                if to.to_ascii_lowercase().ends_with(".png") {
                    media::sticker_to_png(&std::fs::read(&src)?, Path::new(&to))?;
                } else {
                    std::fs::copy(src, &to).with_context(|| format!("save to {to}"))?;
                }
                Ok(Value::Null)
            }

            // ── voice notes ──
            Request::RecordStart => self.record_start(),
            Request::RecordStop { chat, reply_to } => self.record_stop(chat, reply_to),

            // ── settings ──
            Request::SetSettings { settings } => {
                self.store.set_meta("settings", &settings)?;
                *self.settings.write().unwrap_or_else(|e| e.into_inner()) = settings.clone();
                self.emit(UiEvent::Settings { settings });
                Ok(Value::Null)
            }
            Request::SetPrivacy { privacy } => {
                self.net().exec(NetOp::PrivacySet { privacy: privacy.clone() }).await?;
                self.set_privacy(privacy);
                Ok(Value::Null)
            }
            Request::Storage => {
                let db = ["app.db", "app.db-wal", "wa.db", "wa.db-wal"]
                    .iter()
                    .filter_map(|f| std::fs::metadata(self.root.join(f)).ok())
                    .map(|m| m.len())
                    .sum::<u64>();
                val(json!({
                    "media": self.media.usage(),
                    "database": db,
                    "messages": self.store.message_count(),
                    "chats": lock(&self.chats).len(),
                }))
            }
            Request::ClearCache => {
                let freed = self.clear_cache();
                val(json!({ "freed": freed }))
            }

            // ── calls ──
            Request::RejectCall { id, from } => {
                self.emit(UiEvent::Call { id: id.clone(), from: from.clone(), video: false, ringing: false });
                self.fire(NetOp::RejectCall { id, from });
                Ok(Value::Null)
            }
        }
    }

    /// Like [`fire`], but failures are only logged (presence, typing, …).
    fn fire_quiet(self: &Arc<Self>, op: NetOp) {
        let net = self.net();
        tokio::spawn(async move {
            if let Err(e) = net.exec(op).await {
                log::debug!("background op failed: {e:#}");
            }
        });
    }

    /// Asks the network for older messages, at most once per chat per oldest message.
    fn ask_history(self: &Arc<Self>, chat: &str, oldest: Option<Message>) {
        let key = oldest.as_ref().map(|m| m.ts).unwrap_or(0);
        {
            let mut asked = lock(&self.history_asked);
            if asked.get(chat) == Some(&key) {
                return;
            }
            asked.insert(chat.to_string(), key);
        }
        self.fire_quiet(NetOp::History { chat: chat.to_string(), oldest: oldest.map(Box::new) });
    }

    /// Deletes downloaded incoming media; it can be fetched again on demand.
    fn clear_cache(&self) -> u64 {
        let before = self.media.usage();
        for chat in self.all_chats() {
            let mut cursor = None;
            loop {
                let page = self.store.gallery(&chat.id, &["image", "video", "gif", "document", "audio", "voice"], cursor, 200);
                let Some(last) = page.last() else { break };
                cursor = Some(crate::store::Cursor { ts: last.ts, seq: last.seq });
                for m in &page {
                    let Some(media) = m.content.media() else { continue };
                    if m.from_me || media.state != MediaState::Ready {
                        continue;
                    }
                    if let Some(p) = media.token.as_deref().and_then(|t| self.media.path(t)) {
                        let _ = std::fs::remove_file(p);
                    }
                    self.message_mut(&m.chat, &m.id, |m| {
                        if let Some(media) = m.content.media_mut() {
                            media.token = None;
                            media.state = MediaState::Remote;
                        }
                    });
                }
            }
        }
        before.saturating_sub(self.media.usage())
    }

    fn import_avatar(&self, file: Option<&Attachment>) -> Result<Option<String>> {
        let Some(file) = file else { return Ok(None) };
        let (tmp, _) = self.media.import(file)?;
        let path = self.media.path(&tmp).ok_or_else(|| anyhow!("bad path"))?;
        let jpeg = media::to_avatar(&path);
        let _ = std::fs::remove_file(&path);
        Ok(Some(self.media.write("a", "jpg", &jpeg?)?))
    }

    fn avatar_attachment(&self, token: &Option<String>) -> Option<Attachment> {
        let path = self.media.path(token.as_deref()?)?;
        Some(Attachment {
            path: Some(path.to_string_lossy().into_owned()),
            name: "avatar.jpg".into(),
            mime: "image/jpeg".into(),
            ..Default::default()
        })
    }

    // ───────────────────────────── groups & communities ─────────────────────────────

    async fn group_op(self: &Arc<Self>, op: GroupOp) -> Result<Value> {
        let me = self.me();
        match op.clone() {
            GroupOp::Create { subject, participants, parent } => {
                if subject.trim().is_empty() {
                    bail!("Give the group a name");
                }
                let res = self.net().exec(NetOp::Group(op)).await?;
                let id = res.get("id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Group was not created"))?.to_string();
                if self.store.group(&id).is_none() {
                    let mut ps = vec![Participant { id: me.clone(), role: Role::Owner }];
                    ps.extend(participants.iter().map(|j| Participant { id: j.clone(), role: Role::Member }));
                    self.ingest_group(Group {
                        id: id.clone(),
                        subject: subject.trim().to_string(),
                        owner: Some(me),
                        created_at: now_ms(),
                        participants: ps,
                        members_can_add: true,
                        joined: true,
                        parent,
                        ..Default::default()
                    });
                }
                val(json!({ "id": id }))
            }
            GroupOp::Refresh { .. } | GroupOp::InviteInfo { .. } => self.net().exec(NetOp::Group(op)).await,
            GroupOp::Subject { id, text } => {
                if text.trim().is_empty() {
                    bail!("The name can't be empty");
                }
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| g.subject = text.trim().to_string());
                Ok(Value::Null)
            }
            GroupOp::Description { id, text } => {
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| g.description = if text.trim().is_empty() { None } else { Some(text) });
                Ok(Value::Null)
            }
            GroupOp::Setting { id, key, on } => {
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| match key.as_str() {
                    "announce" => g.announce = on,
                    "locked" => g.locked = on,
                    "approval" => g.approval = on,
                    "membersCanAdd" => g.members_can_add = on,
                    _ => {}
                });
                Ok(Value::Null)
            }
            GroupOp::Participants { id, action, jids } => {
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| match action.as_str() {
                    "add" => {
                        for j in &jids {
                            if g.role_of(j).is_none() {
                                g.participants.push(Participant { id: j.clone(), role: Role::Member });
                            }
                        }
                    }
                    "remove" => g.participants.retain(|p| !jids.contains(&p.id)),
                    "promote" | "demote" => {
                        for p in g.participants.iter_mut().filter(|p| jids.contains(&p.id) && p.role != Role::Owner) {
                            p.role = if action == "promote" { Role::Admin } else { Role::Member };
                        }
                    }
                    _ => {}
                });
                Ok(Value::Null)
            }
            GroupOp::Invite { id, .. } => {
                let res = self.net().exec(NetOp::Group(op)).await?;
                let code = res.get("code").and_then(|v| v.as_str()).map(String::from);
                self.group_mut(&id, |g| g.invite_code = code.clone());
                val(json!({ "code": code }))
            }
            GroupOp::Join { .. } => self.net().exec(NetOp::Group(op)).await,
            GroupOp::Leave { id } => {
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| {
                    g.joined = false;
                    g.participants.retain(|p| p.id != me);
                });
                Ok(Value::Null)
            }
            GroupOp::RequestAction { id, jids, approve } => {
                self.net().exec(NetOp::Group(op)).await?;
                self.group_mut(&id, |g| {
                    g.pending.retain(|j| !jids.contains(j));
                    if approve {
                        for j in &jids {
                            if g.role_of(j).is_none() {
                                g.participants.push(Participant { id: j.clone(), role: Role::Member });
                            }
                        }
                    }
                });
                Ok(Value::Null)
            }
            GroupOp::Icon { id, file } => {
                let token = self.import_avatar(file.as_ref())?;
                self.net().exec(NetOp::Group(GroupOp::Icon { id: id.clone(), file: self.avatar_attachment(&token) })).await?;
                self.group_mut(&id, |g| g.avatar = token.clone());
                self.chat_mut(&id, |c| c.avatar = token);
                Ok(Value::Null)
            }
        }
    }

    async fn community_op(self: &Arc<Self>, op: CommunityOp) -> Result<Value> {
        let me = self.me();
        match op.clone() {
            CommunityOp::Create { name, description } => {
                if name.trim().is_empty() {
                    bail!("Give the community a name");
                }
                let res = self.net().exec(NetOp::Community(op)).await?;
                let id = res.get("id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Community was not created"))?.to_string();
                if self.store.group(&id).is_none() {
                    let owner = vec![Participant { id: me.clone(), role: Role::Owner }];
                    self.ingest_group(Group {
                        id: id.clone(),
                        subject: name.trim().to_string(),
                        description: if description.trim().is_empty() { None } else { Some(description) },
                        owner: Some(me.clone()),
                        created_at: now_ms(),
                        participants: owner.clone(),
                        is_community: true,
                        joined: true,
                        ..Default::default()
                    });
                    if let Some(ann) = res.get("announcement").and_then(|v| v.as_str()) {
                        self.ingest_group(Group {
                            id: ann.to_string(),
                            subject: name.trim().to_string(),
                            owner: Some(me),
                            created_at: now_ms(),
                            participants: owner,
                            announce: true,
                            is_announcement: true,
                            parent: Some(id.clone()),
                            joined: true,
                            ..Default::default()
                        });
                    }
                }
                val(json!({ "id": id }))
            }
            CommunityOp::Refresh { .. } => self.net().exec(NetOp::Community(op)).await,
            CommunityOp::Link { id, groups } => {
                self.net().exec(NetOp::Community(op)).await?;
                for g in &groups {
                    self.group_mut(g, |x| x.parent = Some(id.clone()));
                }
                Ok(Value::Null)
            }
            CommunityOp::Unlink { groups, .. } => {
                self.net().exec(NetOp::Community(op)).await?;
                for g in &groups {
                    self.group_mut(g, |x| x.parent = None);
                }
                Ok(Value::Null)
            }
            CommunityOp::JoinSubgroup { group, .. } => {
                self.net().exec(NetOp::Community(op)).await?;
                self.group_mut(&group, |g| {
                    g.joined = true;
                    if g.role_of(&me).is_none() {
                        g.participants.push(Participant { id: me.clone(), role: Role::Member });
                    }
                });
                Ok(Value::Null)
            }
            CommunityOp::Deactivate { id } => {
                self.net().exec(NetOp::Community(op)).await?;
                for g in self.store.groups().into_iter().filter(|g| g.parent.as_deref() == Some(id.as_str())) {
                    if g.is_announcement {
                        self.remove_chat(&g.id);
                        self.remove_group(&g.id);
                    } else {
                        self.group_mut(&g.id, |x| x.parent = None);
                    }
                }
                self.remove_group(&id);
                Ok(Value::Null)
            }
        }
    }

    // ───────────────────────────── stickers ─────────────────────────────

    /// Creates or updates a library sticker and tells the UI.
    pub fn ingest_sticker(&self, id: &str, f: impl FnOnce(&mut Sticker)) {
        if id.is_empty() {
            return;
        }
        match self.store.update_sticker(id, f) {
            Ok(_) => self.emit(UiEvent::Stickers),
            Err(e) => log::warn!("sticker update failed: {e:#}"),
        }
    }

    /// A sticker was sent from one of the user's devices at `ts`.
    pub fn sticker_sent(&self, id: &str, ts: i64, remote: Option<RemoteSticker>, animated: bool) {
        let fresh = remote.is_some();
        self.ingest_sticker(id, |s| {
            s.used = s.used.max(ts);
            s.animated |= animated;
            // A just-sent copy has a live path; an old one may have expired.
            if fresh && (s.remote.is_none() || s.token.is_none() || s.missing) {
                s.remote = remote;
                s.missing = false;
            }
        });
        if fresh {
            lock(&self.sticker_jobs).remove(id);
        }
    }

    /// A message carrying a sticker arrived. If that sticker is in the library
    /// but couldn't be downloaded, the message's newer copy is used instead.
    pub fn sticker_seen(&self, id: &str, remote: RemoteSticker) {
        let Some(s) = self.store.sticker(id) else { return };
        if s.token.is_some() && !s.missing {
            return;
        }
        self.ingest_sticker(id, |s| {
            s.remote = Some(remote);
            s.missing = false;
        });
        lock(&self.sticker_jobs).remove(id);
    }

    /// Another device removed a sticker from "Recent" (if not sent again since `before`).
    pub fn sticker_unrecent(&self, id: &str, before: Option<i64>) {
        if self.store.sticker(id).is_some_and(|s| s.used > 0 && before.is_none_or(|b| s.used <= b)) {
            self.ingest_sticker(id, |s| s.used = 0);
        }
    }

    /// The library, starting downloads for stickers whose file isn't here yet.
    fn sticker_list(self: &Arc<Self>) -> Vec<Sticker> {
        let list = self.store.stickers();
        let absent = |s: &&Sticker| {
            s.remote.is_some() && !s.missing && s.token.as_deref().and_then(|t| self.media.path(t)).is_none_or(|p| !p.exists())
        };
        // Every favourite, plus the recents the picker actually shows.
        let missing = list
            .iter()
            .filter(|s| s.favorite)
            .filter(absent)
            .chain(list.iter().filter(|s| s.used > 0).take(RECENT_STICKERS).filter(absent))
            .cloned()
            .collect::<Vec<_>>();
        for s in missing {
            self.fetch_sticker(s);
        }
        list
    }

    fn fetch_sticker(self: &Arc<Self>, s: Sticker) {
        if !lock(&self.sticker_jobs).insert(s.id.clone()) {
            return;
        }
        let app = self.clone();
        tokio::spawn(async move {
            let id = s.id.clone();
            match app.net().exec(NetOp::StickerDownload { sticker: Box::new(s) }).await {
                Ok(v) => {
                    let Some(token) = v.get("token").and_then(|t| t.as_str()).map(String::from) else {
                        // Gone for good. Stays in `sticker_jobs`; a fresh copy clears both.
                        app.ingest_sticker(&id, |s| s.missing = true);
                        return;
                    };
                    let remote = v.get("remote").and_then(|r| serde_json::from_value::<RemoteSticker>(r.clone()).ok());
                    let animated = app
                        .media
                        .path(&token)
                        .and_then(|p| std::fs::read(p).ok())
                        .is_some_and(|b| media::is_animated_webp(&b));
                    app.ingest_sticker(&id, |s| {
                        s.token = Some(token);
                        s.animated = animated;
                        s.missing = false;
                        // The copy that worked, so favouriting it syncs a usable reference.
                        if remote.is_some() {
                            s.remote = remote;
                        }
                    });
                    lock(&app.sticker_jobs).remove(&id);
                }
                // A network problem or an offline phone: tried again next time the picker opens.
                Err(e) => {
                    log::info!("sticker download failed: {e:#}");
                    lock(&app.sticker_jobs).remove(&id);
                }
            }
        });
    }

    // ───────────────────────────── voice notes ─────────────────────────────

    #[cfg(feature = "voice")]
    fn record_start(self: &Arc<Self>) -> Result<Value> {
        let weak = Arc::downgrade(self);
        let rec = crate::voice::Recorder::start(move |level, elapsed| {
            if let Some(app) = weak.upgrade() {
                app.emit(UiEvent::Recording { level, elapsed });
            }
        })?;
        *lock(&self.recorder) = Some(rec);
        Ok(Value::Null)
    }

    #[cfg(feature = "voice")]
    fn record_stop(self: &Arc<Self>, chat: Option<Jid>, reply_to: Option<String>) -> Result<Value> {
        let Some(rec) = lock(&self.recorder).take() else { bail!("Not recording") };
        let Some(chat) = chat else {
            drop(rec);
            return Ok(Value::Null);
        };
        let take = rec.finish()?;
        if take.duration_ms < 500 {
            bail!("Hold on a little longer to record a voice message");
        }
        let chat_row = self.ensure_chat(&chat);
        let token = self.media.write("m", "ogg", &take.ogg)?;
        let media = Media {
            token: Some(token),
            mime: "audio/ogg; codecs=opus".into(),
            size: take.ogg.len() as u64,
            state: MediaState::Ready,
        };
        let content = Content::Audio {
            media,
            duration: (take.duration_ms as f64 / 1000.0).round().max(1.0) as u32,
            voice: true,
            waveform: take.waveform,
        };
        let mut m = self.base_message(&chat_row, content);
        m.quote = reply_to.and_then(|id| self.store.message(&chat, &id)).map(|q| q.quote());
        val(self.dispatch(m))
    }

    #[cfg(not(feature = "voice"))]
    fn record_start(self: &Arc<Self>) -> Result<Value> {
        bail!("Voice recording is not available in this build")
    }

    #[cfg(not(feature = "voice"))]
    fn record_stop(self: &Arc<Self>, _chat: Option<Jid>, _reply_to: Option<String>) -> Result<Value> {
        bail!("Voice recording is not available in this build")
    }
}
