//! Data shapes shared between the Rust core and the UI.
//!
//! Everything here serialises to camelCase JSON; `ui/src/lib/types.ts` mirrors it.

use serde::{Deserialize, Serialize};

/// A WhatsApp address in its string form, e.g. `15551234567@s.whatsapp.net`
/// or `120363012345678901@g.us`.
pub type Jid = String;

pub const USER_SERVER: &str = "s.whatsapp.net";
pub const GROUP_SERVER: &str = "g.us";

pub fn is_group(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

/// Strips the device/agent part so two devices of the same user compare equal.
pub fn bare(jid: &str) -> String {
    match jid.split_once('@') {
        Some((user, server)) if server == USER_SERVER || server == "lid" => {
            let user = user.split([':', '.']).next().unwrap_or(user);
            format!("{user}@{server}")
        }
        _ => jid.to_string(),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ───────────────────────────── chats ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ChatKind {
    #[default]
    Direct,
    Group,
    /// The announcement group of a community.
    Announcement,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Chat {
    pub id: Jid,
    pub kind: ChatKind,
    pub name: String,
    pub avatar: Option<String>,
    pub last: Option<Preview>,
    /// Sort timestamp (ms). Usually the last message time.
    pub ts: i64,
    pub unread: u32,
    pub unread_mentions: u32,
    pub marked_unread: bool,
    /// 0 when not pinned, otherwise the time it was pinned.
    pub pinned: i64,
    pub archived: bool,
    /// 0 = not muted, -1 = forever, otherwise expiry (ms).
    pub muted_until: i64,
    pub favorite: bool,
    pub draft: Option<String>,
    /// Disappearing-message timer in seconds (0 = off).
    pub ephemeral: u32,
    /// Parent community, when this group belongs to one.
    pub community: Option<Jid>,
    /// Only admins may send (and we are not one), or we left the group.
    pub read_only: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Preview {
    pub id: String,
    pub text: String,
    /// Content kind (`text`, `image`, `sticker`, …) so the UI can pick an icon.
    pub kind: String,
    pub from_me: bool,
    pub sender: Option<Jid>,
    pub status: Status,
    pub ts: i64,
}

// ───────────────────────────── people ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Contact {
    pub id: Jid,
    /// Name from the user's address book.
    pub name: Option<String>,
    /// Name the contact chose for themselves.
    pub push_name: Option<String>,
    /// Verified business name.
    pub verified_name: Option<String>,
    pub about: Option<String>,
    pub avatar: Option<String>,
    pub blocked: bool,
    pub is_me: bool,
    pub is_business: bool,
}

impl Contact {
    pub fn display(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.verified_name.clone())
            .or_else(|| self.push_name.clone())
            .unwrap_or_else(|| phone_of(&self.id))
    }
}

/// Renders the user part of a JID as a phone number.
pub fn phone_of(jid: &str) -> String {
    let user = jid.split('@').next().unwrap_or(jid);
    let user = user.split([':', '.']).next().unwrap_or(user);
    if jid.ends_with("@lid") || user.is_empty() || !user.chars().all(|c| c.is_ascii_digit()) {
        return user.to_string();
    }
    format!("+{user}")
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    #[default]
    Member,
    Admin,
    Owner,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Participant {
    pub id: Jid,
    pub role: Role,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Group {
    pub id: Jid,
    pub subject: String,
    pub description: Option<String>,
    pub owner: Option<Jid>,
    pub created_at: i64,
    pub participants: Vec<Participant>,
    /// Only admins can send messages.
    pub announce: bool,
    /// Only admins can edit group info.
    pub locked: bool,
    /// New members need admin approval.
    pub approval: bool,
    /// Whether every member (not only admins) may add people.
    pub members_can_add: bool,
    pub ephemeral: u32,
    pub invite_code: Option<String>,
    /// This row is a community parent, not a chat.
    pub is_community: bool,
    pub is_announcement: bool,
    pub parent: Option<Jid>,
    /// Join requests awaiting approval.
    pub pending: Vec<Jid>,
    /// We are still a member.
    pub joined: bool,
    pub avatar: Option<String>,
    /// Member count for groups we have not joined (community subgroups).
    pub size: u32,
}

impl Group {
    pub fn role_of(&self, jid: &str) -> Option<Role> {
        self.participants.iter().find(|p| p.id == jid).map(|p| p.role)
    }
}

// ───────────────────────────── messages ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Failed,
    #[default]
    Pending,
    Sent,
    Delivered,
    Read,
    Played,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MediaState {
    /// Known on the server, not fetched yet.
    #[default]
    Remote,
    Downloading,
    Ready,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Media {
    /// Path token inside the media directory once the file is local.
    pub token: Option<String>,
    pub mime: String,
    pub size: u64,
    pub state: MediaState,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub site: String,
    /// Base64 JPEG.
    pub thumb: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PollOption {
    pub name: String,
    pub voters: Vec<Jid>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactCard {
    pub name: String,
    pub vcard: String,
    pub phone: Option<String>,
    pub jid: Option<Jid>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct EventResponses {
    pub going: Vec<Jid>,
    pub maybe: Vec<Jid>,
    pub declined: Vec<Jid>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Content {
    Text {
        text: String,
        #[serde(default)]
        preview: Option<LinkPreview>,
    },
    Image {
        media: Media,
        #[serde(default)]
        caption: String,
        width: u32,
        height: u32,
        #[serde(default)]
        thumb: Option<String>,
    },
    Video {
        media: Media,
        #[serde(default)]
        caption: String,
        width: u32,
        height: u32,
        duration: u32,
        #[serde(default)]
        gif: bool,
        #[serde(default)]
        thumb: Option<String>,
    },
    Audio {
        media: Media,
        duration: u32,
        /// Voice note (push-to-talk) rather than a music/audio file.
        #[serde(default)]
        voice: bool,
        /// 0–100 amplitude samples.
        #[serde(default)]
        waveform: Vec<u8>,
    },
    Document {
        media: Media,
        name: String,
        #[serde(default)]
        caption: String,
        #[serde(default)]
        pages: u32,
        #[serde(default)]
        thumb: Option<String>,
    },
    Sticker {
        media: Media,
        width: u32,
        height: u32,
        #[serde(default)]
        animated: bool,
    },
    Location {
        lat: f64,
        lng: f64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        address: String,
        #[serde(default)]
        live: bool,
        #[serde(default)]
        thumb: Option<String>,
    },
    Contacts {
        cards: Vec<ContactCard>,
    },
    Poll {
        question: String,
        options: Vec<PollOption>,
        /// 0 = any number of options, 1 = single choice.
        selectable: u32,
    },
    Event {
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        location: String,
        start: i64,
        #[serde(default)]
        end: i64,
        #[serde(default)]
        canceled: bool,
        #[serde(default)]
        responses: EventResponses,
    },
    /// Group/system notices rendered as a centred pill.
    System {
        event: String,
        #[serde(default)]
        actor: Option<Jid>,
        #[serde(default)]
        targets: Vec<Jid>,
        #[serde(default)]
        text: String,
    },
    Call {
        video: bool,
        missed: bool,
        #[serde(default)]
        duration: u32,
    },
    Deleted,
    Unsupported {
        #[serde(default)]
        note: String,
    },
}

impl Content {
    pub fn kind(&self) -> &'static str {
        match self {
            Content::Text { .. } => "text",
            Content::Image { .. } => "image",
            Content::Video { gif: true, .. } => "gif",
            Content::Video { .. } => "video",
            Content::Audio { voice: true, .. } => "voice",
            Content::Audio { .. } => "audio",
            Content::Document { .. } => "document",
            Content::Sticker { .. } => "sticker",
            Content::Location { .. } => "location",
            Content::Contacts { .. } => "contacts",
            Content::Poll { .. } => "poll",
            Content::Event { .. } => "event",
            Content::System { .. } => "system",
            Content::Call { .. } => "call",
            Content::Deleted => "deleted",
            Content::Unsupported { .. } => "unsupported",
        }
    }

    /// Searchable / previewable text of the message.
    pub fn text(&self) -> &str {
        match self {
            Content::Text { text, .. } => text,
            Content::Image { caption, .. }
            | Content::Video { caption, .. }
            | Content::Document { caption, .. } => caption,
            Content::Poll { question, .. } => question,
            Content::Event { name, .. } => name,
            Content::Location { name, .. } => name,
            Content::System { text, .. } => text,
            _ => "",
        }
    }

    pub fn set_text(&mut self, new: String) {
        match self {
            Content::Text { text, .. } => *text = new,
            Content::Image { caption, .. }
            | Content::Video { caption, .. }
            | Content::Document { caption, .. } => *caption = new,
            _ => {}
        }
    }

    pub fn media(&self) -> Option<&Media> {
        match self {
            Content::Image { media, .. }
            | Content::Video { media, .. }
            | Content::Audio { media, .. }
            | Content::Document { media, .. }
            | Content::Sticker { media, .. } => Some(media),
            _ => None,
        }
    }

    pub fn media_mut(&mut self) -> Option<&mut Media> {
        match self {
            Content::Image { media, .. }
            | Content::Video { media, .. }
            | Content::Audio { media, .. }
            | Content::Document { media, .. }
            | Content::Sticker { media, .. } => Some(media),
            _ => None,
        }
    }

    /// One-line summary used in chat list rows, reply quotes and notifications.
    pub fn summary(&self) -> String {
        let cap = |label: &str, caption: &str| {
            if caption.is_empty() { label.to_string() } else { caption.to_string() }
        };
        match self {
            Content::Text { text, .. } => text.clone(),
            Content::Image { caption, .. } => cap("Photo", caption),
            Content::Video { gif: true, caption, .. } => cap("GIF", caption),
            Content::Video { caption, .. } => cap("Video", caption),
            Content::Audio { voice: true, .. } => "Voice message".into(),
            Content::Audio { .. } => "Audio".into(),
            Content::Document { name, caption, .. } => cap(name, caption),
            Content::Sticker { .. } => "Sticker".into(),
            Content::Location { live: true, .. } => "Live location".into(),
            Content::Location { name, .. } => cap("Location", name),
            Content::Contacts { cards } => match cards.as_slice() {
                [one] => one.name.clone(),
                many => format!("{} contacts", many.len()),
            },
            Content::Poll { question, .. } => question.clone(),
            Content::Event { name, .. } => name.clone(),
            Content::System { event, text, .. } => match event.as_str() {
                "create" => "Group created".into(),
                "add" => "Members were added".into(),
                "remove" => "A member was removed".into(),
                "leave" => "Someone left the group".into(),
                "join" => "Joined using an invite link".into(),
                "promote" => "A member is now an admin".into(),
                "demote" => "A member is no longer an admin".into(),
                "subject" => format!("Group name changed to “{text}”"),
                "description" => "Group description changed".into(),
                "icon" => "Group picture changed".into(),
                "pin" => "A message was pinned".into(),
                "ephemeral" => "Disappearing messages were updated".into(),
                "announce" | "locked" | "approval" | "membersCanAdd" => "Group settings changed".into(),
                _ => text.clone(),
            },
            Content::Call { video: true, missed: true, .. } => "Missed video call".into(),
            Content::Call { missed: true, .. } => "Missed voice call".into(),
            Content::Call { video: true, .. } => "Video call".into(),
            Content::Call { .. } => "Voice call".into(),
            Content::Deleted => "This message was deleted".into(),
            Content::Unsupported { .. } => "Unsupported message".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Reaction {
    pub emoji: String,
    pub sender: Jid,
    pub ts: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Quote {
    pub id: String,
    pub sender: Jid,
    pub text: String,
    pub kind: String,
    pub thumb: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub chat: Jid,
    /// Local insertion order; breaks ties between equal timestamps.
    #[serde(default)]
    pub seq: i64,
    pub sender: Jid,
    pub from_me: bool,
    pub ts: i64,
    pub content: Content,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub quote: Option<Quote>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub mentions: Vec<Jid>,
    #[serde(default)]
    pub starred: bool,
    /// Expiry (ms) of the in-chat pin; 0 when not pinned.
    #[serde(default)]
    pub pinned_until: i64,
    /// Time of the last edit; 0 when never edited.
    #[serde(default)]
    pub edited: i64,
    /// 0 = original, 1 = forwarded, ≥5 = forwarded many times.
    #[serde(default)]
    pub forwarded: u32,
    #[serde(default)]
    pub view_once: bool,
    /// When a disappearing message is due to vanish; 0 = never.
    #[serde(default)]
    pub expires: i64,
    /// Photos/videos sent together share an album id.
    #[serde(default)]
    pub album: Option<String>,
}

impl Message {
    pub fn preview(&self) -> Preview {
        Preview {
            id: self.id.clone(),
            text: truncate(&self.content.summary(), 140),
            kind: self.content.kind().to_string(),
            from_me: self.from_me,
            sender: Some(self.sender.clone()),
            status: self.status,
            ts: self.ts,
        }
    }

    pub fn quote(&self) -> Quote {
        let thumb = match &self.content {
            Content::Image { thumb, .. }
            | Content::Video { thumb, .. }
            | Content::Document { thumb, .. }
            | Content::Location { thumb, .. } => thumb.clone(),
            _ => None,
        };
        Quote {
            id: self.id.clone(),
            sender: self.sender.clone(),
            text: truncate(&self.content.summary(), 240),
            kind: self.content.kind().to_string(),
            thumb,
        }
    }

    pub fn is_system(&self) -> bool {
        matches!(self.content, Content::System { .. } | Content::Call { .. })
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Per-recipient delivery details for the "Message info" sheet.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Receipt {
    pub user: Jid,
    pub delivered: i64,
    pub read: i64,
    pub played: i64,
}

// ───────────────────────────── session & settings ─────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Session {
    /// Nothing decided yet: the UI shows the welcome screen.
    #[default]
    Welcome,
    /// Waiting for the phone to scan / enter a code.
    Pairing {
        qr: Option<String>,
        code: Option<String>,
        /// When the current QR/code stops being valid (ms).
        expires: i64,
    },
    /// Paired; first sync in progress.
    Syncing { progress: u8 },
    Connecting,
    Online,
    /// Lost the connection; retrying.
    Offline,
    /// The session ended and needs a fresh pairing.
    LoggedOut { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// `mono` (white on dark, black on light) or a colour preset.
    pub accent: String,
    /// Interface typeface: `system` or one of the bundled fonts.
    pub font: String,
    /// Bumped when defaults change, so stored settings can be migrated once.
    /// Missing in older saves, so it reads as 0 there rather than the current value.
    #[serde(default)]
    pub design: u32,
    /// UI scale multiplier applied to the base font size.
    pub text_scale: f32,
    pub compact: bool,
    pub enter_sends: bool,
    pub notifications: bool,
    pub notification_preview: bool,
    pub notification_sound: bool,
    pub auto_images: bool,
    pub auto_audio: bool,
    pub auto_video: bool,
    pub auto_documents: bool,
    pub link_previews: bool,
    pub close_to_tray: bool,
    pub autostart: bool,
    pub spellcheck: bool,
    pub sidebar_width: u32,
    /// Quick-reaction emoji shown on hover.
    pub quick_reactions: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            accent: "mono".into(),
            font: "system".into(),
            design: Settings::DESIGN,
            text_scale: 1.0,
            compact: false,
            enter_sends: true,
            notifications: true,
            notification_preview: true,
            notification_sound: true,
            auto_images: true,
            auto_audio: true,
            auto_video: false,
            auto_documents: false,
            link_previews: true,
            close_to_tray: true,
            autostart: false,
            spellcheck: true,
            sidebar_width: 348,
            quick_reactions: ["👍", "❤️", "😂", "😮", "😢", "🙏"].map(String::from).to_vec(),
        }
    }
}

impl Settings {
    /// Current design revision. 2: monochrome accent and system font by default.
    pub const DESIGN: u32 = 2;

    /// Brings settings saved by an older version up to the current defaults,
    /// leaving any choice the user made away from the old defaults alone.
    pub fn migrate(mut self) -> (Settings, bool) {
        if self.design >= Self::DESIGN {
            return (self, false);
        }
        if self.accent == "tidal" || self.accent.is_empty() {
            self.accent = "mono".into();
        }
        if self.font.is_empty() {
            self.font = "system".into();
        }
        self.design = Self::DESIGN;
        (self, true)
    }
}

/// Server-side privacy choices (`all`, `contacts`, `none`, …).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Privacy {
    pub last_seen: String,
    pub online: String,
    pub profile_photo: String,
    pub about: String,
    pub read_receipts: bool,
    pub group_add: String,
    pub default_ephemeral: u32,
}

impl Default for Privacy {
    fn default() -> Self {
        Privacy {
            last_seen: "contacts".into(),
            online: "all".into(),
            profile_photo: "contacts".into(),
            about: "contacts".into(),
            read_receipts: true,
            group_add: "all".into(),
            default_ephemeral: 0,
        }
    }
}

/// A sticker in the user's library (recents and favourites).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Sticker {
    /// Base64 SHA-256 of the sticker file: the id WhatsApp syncs favourites and
    /// recents by. Stickers saved by older versions use their file token.
    pub id: String,
    /// Local file, once downloaded.
    pub token: Option<String>,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    pub favorite: bool,
    /// When it was last sent (ms); 0 keeps it out of "Recent".
    pub used: i64,
    /// Where WhatsApp keeps the file, for downloading and for syncing favourites.
    pub remote: Option<RemoteSticker>,
    /// WhatsApp no longer has the file and the phone couldn't upload it again.
    /// Cleared when a fresh copy turns up (the sticker is sent or favourited again).
    pub missing: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteSticker {
    pub direct_path: String,
    /// Base64.
    pub media_key: String,
    /// Base64.
    pub file_enc_sha256: String,
    pub file_length: u64,
    pub mimetype: String,
}

/// Base64 SHA-256 of a file, the way WhatsApp identifies stickers.
pub fn file_hash(bytes: &[u8]) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_strips_device() {
        assert_eq!(bare("15551234567:12@s.whatsapp.net"), "15551234567@s.whatsapp.net");
        assert_eq!(bare("15551234567@s.whatsapp.net"), "15551234567@s.whatsapp.net");
        assert_eq!(bare("120363@g.us"), "120363@g.us");
        assert_eq!(bare("9876:3@lid"), "9876@lid");
    }

    #[test]
    fn phone_formatting() {
        assert_eq!(phone_of("15551234567@s.whatsapp.net"), "+15551234567");
        assert_eq!(phone_of("9876@lid"), "9876");
    }

    #[test]
    fn content_round_trip() {
        let c = Content::Video {
            media: Media::default(),
            caption: "hi".into(),
            width: 1,
            height: 2,
            duration: 3,
            gif: true,
            thumb: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"type\":\"video\""));
        let back: Content = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind(), "gif");
        assert_eq!(back.summary(), "hi");
    }

    #[test]
    fn settings_migrate_once() {
        let old: Settings = serde_json::from_str(r#"{"theme":"dark","accent":"tidal"}"#).unwrap();
        let (s, changed) = old.migrate();
        assert!(changed);
        assert_eq!((s.accent.as_str(), s.font.as_str(), s.theme.as_str()), ("mono", "system", "dark"));
        let (again, changed) = s.clone().migrate();
        assert!(!changed && again.accent == "mono");
        let custom: Settings = serde_json::from_str(r#"{"accent":"coral"}"#).unwrap();
        assert_eq!(custom.migrate().0.accent, "coral", "a colour the user picked is kept");
        assert!(!Settings::default().migrate().1);
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("héllo wörld", 5), "héllo…");
        assert_eq!(truncate("ok", 5), "ok");
    }
}
