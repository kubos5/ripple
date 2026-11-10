//! The command/event surface between the UI and the core, and between the
//! core and a network backend (live WhatsApp or the demo).

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::model::*;
use crate::store::Cursor;

// ───────────────────────────── UI → core ─────────────────────────────

/// A file handed over by the UI: either a path on disk (file dialog, drag and
/// drop) or raw bytes (clipboard paste, canvas output).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Attachment {
    pub path: Option<String>,
    /// Base64 payload when there is no path.
    pub data: Option<String>,
    pub name: String,
    pub mime: String,
    /// Filled in by the UI for videos (the core does not decode video).
    pub width: u32,
    pub height: u32,
    pub duration: u32,
    pub thumb: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Outgoing {
    Text {
        text: String,
        #[serde(default)]
        preview: Option<LinkPreview>,
    },
    /// `kind` is one of image, video, gif, audio, voice, document, sticker.
    Media {
        file: Attachment,
        kind: String,
        #[serde(default)]
        caption: String,
        #[serde(default)]
        view_once: bool,
        #[serde(default)]
        album: Option<String>,
    },
    /// Re-send a sticker already in the library.
    Sticker { token: String },
    Location {
        lat: f64,
        lng: f64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        address: String,
    },
    Contacts { jids: Vec<Jid> },
    Poll { question: String, options: Vec<String>, selectable: u32 },
    Event {
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        location: String,
        start: i64,
        #[serde(default)]
        end: i64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GroupOp {
    Create {
        subject: String,
        participants: Vec<Jid>,
        #[serde(default)]
        parent: Option<Jid>,
    },
    Refresh { id: Jid },
    Subject { id: Jid, text: String },
    Description { id: Jid, text: String },
    /// `key` is announce, locked, approval or membersCanAdd.
    Setting { id: Jid, key: String, on: bool },
    /// `action` is add, remove, promote or demote.
    Participants { id: Jid, action: String, jids: Vec<Jid> },
    Invite {
        id: Jid,
        #[serde(default)]
        reset: bool,
    },
    InviteInfo { code: String },
    Join { code: String },
    Leave { id: Jid },
    RequestAction { id: Jid, jids: Vec<Jid>, approve: bool },
    Icon { id: Jid, file: Option<Attachment> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CommunityOp {
    Create {
        name: String,
        #[serde(default)]
        description: String,
    },
    Refresh { id: Jid },
    Link { id: Jid, groups: Vec<Jid> },
    Unlink { id: Jid, groups: Vec<Jid> },
    JoinSubgroup { id: Jid, group: Jid },
    Deactivate { id: Jid },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProfileOp {
    Name { text: String },
    About { text: String },
    Photo { file: Option<Attachment> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    // session
    Init,
    Pair {
        #[serde(default)]
        phone: Option<String>,
    },
    Logout,
    DemoStart,
    DemoExit,
    /// Dev/demo helper: `call`, `message`, `typing`.
    DemoTrigger { what: String },
    Focus { on: bool },

    // chats
    OpenChat { chat: Jid },
    CloseChat,
    Messages {
        chat: Jid,
        #[serde(default)]
        before: Option<Cursor>,
        #[serde(default)]
        after: Option<Cursor>,
        #[serde(default)]
        limit: Option<usize>,
    },
    MessagesAround { chat: Jid, id: String },
    MarkRead { chat: Jid },
    MarkUnread { chat: Jid },
    Archive { chat: Jid, on: bool },
    PinChat { chat: Jid, on: bool },
    Mute { chat: Jid, until: i64 },
    Favorite { chat: Jid, on: bool },
    DeleteChat { chat: Jid },
    ClearChat { chat: Jid },
    Draft { chat: Jid, text: String },
    Ephemeral { chat: Jid, secs: u32 },
    Typing { chat: Jid, state: String },
    Subscribe { jid: Jid },
    StartChat { jid: Jid },
    CheckNumber { phone: String },

    // messages
    Send {
        chat: Jid,
        content: Outgoing,
        #[serde(default)]
        reply_to: Option<String>,
        #[serde(default)]
        mentions: Vec<Jid>,
    },
    Retry { chat: Jid, id: String },
    React { chat: Jid, id: String, emoji: String },
    Edit { chat: Jid, id: String, text: String },
    Delete { chat: Jid, ids: Vec<String>, everyone: bool },
    Star { chat: Jid, ids: Vec<String>, on: bool },
    PinMessage { chat: Jid, id: String, secs: u32 },
    Forward { chat: Jid, ids: Vec<String>, to: Vec<Jid> },
    Vote { chat: Jid, id: String, options: Vec<String> },
    Rsvp { chat: Jid, id: String, response: String },
    MessageInfo { chat: Jid, id: String },
    Download { chat: Jid, id: String },
    Played { chat: Jid, id: String },
    SaveMedia { chat: Jid, id: String, to: String },

    // lookups
    Starred {
        #[serde(default)]
        chat: Option<Jid>,
    },
    Pinned { chat: Jid },
    Gallery {
        chat: Jid,
        /// `media`, `docs`, `links` or `audio`.
        tab: String,
        #[serde(default)]
        before: Option<Cursor>,
    },
    Search {
        query: String,
        #[serde(default)]
        chat: Option<Jid>,
    },
    LinkPreview { url: String },

    // people, groups, communities
    SaveContact { jid: Jid, name: String },
    Block { jid: Jid, on: bool },
    Avatar { jid: Jid },
    Group { op: GroupOp },
    Community { op: CommunityOp },
    Profile { op: ProfileOp },

    // stickers
    Stickers,
    StickerFavorite { token: String, on: bool },
    StickerRemove { token: String },
    StickerCreate { file: Attachment },

    // voice notes
    RecordStart,
    RecordStop { chat: Option<Jid>, reply_to: Option<String> },

    // settings
    SetSettings { settings: Settings },
    SetPrivacy { privacy: Privacy },
    Storage,
    ClearCache,

    // calls
    RejectCall { id: String, from: Jid },
}

// ───────────────────────────── core → UI ─────────────────────────────

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiEvent {
    /// The data set changed wholesale (demo entered/left, logged out): re-run `init`.
    Reset,
    Session { session: Session },
    Chat { chat: Chat },
    ChatRemoved { id: Jid },
    /// A message was added or changed. `fresh` marks a brand-new arrival.
    Message { message: Message, fresh: bool },
    MessageRemoved { chat: Jid, id: String },
    /// Older history arrived for a chat; reload the visible page.
    History { chat: Jid },
    Contact { contact: Contact },
    Group { group: Group },
    GroupRemoved { id: Jid },
    Typing { chat: Jid, user: Jid, state: String },
    Presence { jid: Jid, online: bool, last_seen: i64 },
    Settings { settings: Settings },
    Privacy { privacy: Privacy },
    Me { contact: Contact },
    /// Ask the shell to raise a native notification.
    Notify { chat: Jid, id: String, title: String, body: String, avatar: Option<String> },
    Call { id: String, from: Jid, video: bool, ringing: bool },
    Recording { level: u8, elapsed: u32 },
    Progress { chat: Jid, id: String, progress: u8 },
    Toast { tone: String, text: String },
    /// Total unread chats, for the tray/dock badge.
    Badge { count: u32 },
}

// ───────────────────────────── core → network ─────────────────────────────

#[derive(Clone, Debug)]
pub enum ChatAction {
    Archive(bool),
    Pin(bool),
    Mute(i64),
    MarkRead(bool),
    Delete,
    Clear,
}

/// Everything the core may ask a network backend to do.
#[derive(Clone, Debug)]
pub enum NetOp {
    Pair { phone: Option<String> },
    Logout,
    Send { msg: Box<Message> },
    React { target: Box<Message>, emoji: String },
    Edit { target: Box<Message>, text: String },
    Revoke { target: Box<Message> },
    DeleteForMe { targets: Vec<Message> },
    Pin { target: Box<Message>, secs: u32 },
    Star { targets: Vec<Message>, on: bool },
    Vote { target: Box<Message>, options: Vec<String> },
    Rsvp { target: Box<Message>, response: String },
    MarkRead { chat: Jid, msgs: Vec<Message> },
    MarkPlayed { target: Box<Message> },
    Chat { chat: Jid, action: ChatAction, last: Option<Box<Message>> },
    Typing { chat: Jid, state: String },
    Subscribe { jid: Jid },
    Available { on: bool },
    Ephemeral { chat: Jid, secs: u32 },
    Download { target: Box<Message> },
    History { chat: Jid, oldest: Option<Box<Message>> },
    Avatar { jid: Jid },
    CheckNumber { phone: String },
    Block { jid: Jid, on: bool },
    SaveContact { jid: Jid, name: String },
    Group(GroupOp),
    Community(CommunityOp),
    Profile(ProfileOp),
    PrivacyGet,
    PrivacySet { privacy: Privacy },
    RejectCall { id: String, from: Jid },
    DemoTrigger { what: String },
}

/// A network backend. It performs [`NetOp`]s and reports what happens on the
/// wire by calling the `ingest_*` methods on [`App`].
#[async_trait]
pub trait Network: Send + Sync + 'static {
    /// Called once after the app is wired up: connect, or resume a session.
    async fn start(&self, app: Arc<App>) -> Result<()>;
    async fn exec(&self, op: NetOp) -> Result<Value>;
    /// Whether this backend talks to the real service.
    fn is_live(&self) -> bool;
    async fn shutdown(&self) {}
}

/// Placeholder used before the user picks demo or live.
pub struct NoNetwork;

#[async_trait]
impl Network for NoNetwork {
    async fn start(&self, _app: Arc<App>) -> Result<()> {
        Ok(())
    }
    async fn exec(&self, _op: NetOp) -> Result<Value> {
        anyhow::bail!("Not connected")
    }
    fn is_live(&self) -> bool {
        false
    }
}
