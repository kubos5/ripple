//! Translation between WhatsApp's protobuf messages and Ripple's model.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use whatsapp_rust::Jid as WaJid;
use whatsapp_rust::waproto::buffa::{Message as _, MessageField};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::wacore::proto_helpers::MessageExt;

use crate::model::*;

/// Builds a protobuf message field by field. The generated types are
/// non-exhaustive, so struct literals are not allowed outside their crate.
macro_rules! pb {
    ($ty:path { $($field:ident : $value:expr),* $(,)? }) => {{
        #[allow(clippy::field_reassign_with_default)]
        let mut v = <$ty>::default();
        $( v.$field = $value; )*
        v
    }};
}
pub(crate) use pb;

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn unb64(s: &str) -> Vec<u8> {
    B64.decode(s).unwrap_or_default()
}

/// A JID without its device part, as the string Ripple uses for ids.
pub fn jid_str(j: &WaJid) -> String {
    j.to_non_ad_string()
}

pub fn parse_jid(s: &str) -> anyhow::Result<WaJid> {
    s.parse::<WaJid>().map_err(|e| anyhow::anyhow!("invalid address {s}: {e}"))
}

/// What a decrypted message turned out to be, beyond plain content.
#[derive(Debug, Default)]
pub enum Special {
    #[default]
    None,
    Reaction { target: String, emoji: String },
    Revoke { target: String },
    Edit { target: String, text: String },
    PollVote { poll: String, payload: Vec<u8>, iv: Vec<u8> },
    EventResponse { event: String, payload: Vec<u8>, iv: Vec<u8> },
    Pin { target: String, on: bool },
    Ephemeral { secs: u32 },
    /// Protocol plumbing with nothing to show.
    Skip,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub content: Option<Content>,
    pub quote: Option<Quote>,
    pub mentions: Vec<String>,
    pub forwarded: u32,
    pub view_once: bool,
    /// Disappearing timer carried by the message (seconds).
    pub expiration: u32,
    pub album: Option<String>,
    /// Message secret (needed later to decrypt poll votes and event replies).
    pub secret: Option<Vec<u8>>,
    pub special: Special,
}

fn remote(mime: &Option<String>, size: Option<u64>) -> Media {
    Media { token: None, mime: mime.clone().unwrap_or_default(), size: size.unwrap_or(0), state: MediaState::Remote }
}

fn thumb(bytes: &Option<Vec<u8>>) -> Option<String> {
    bytes.as_ref().filter(|b| !b.is_empty()).map(|b| b64(b))
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").trim_start_matches("www.").to_string()
}

/// Pulls a WhatsApp id and phone number out of a vCard.
fn vcard_contact(name: &str, vcard: &str) -> ContactCard {
    let mut phone = None;
    let mut jid = None;
    for line in vcard.lines() {
        if !line.to_ascii_uppercase().starts_with("TEL") {
            continue;
        }
        if let Some(i) = line.find("waid=") {
            let digits: String = line[i + 5..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                jid = Some(format!("{digits}@{USER_SERVER}"));
            }
        }
        if let Some((_, value)) = line.rsplit_once(':') {
            phone = Some(value.trim().to_string());
        }
        break;
    }
    ContactCard { name: name.to_string(), vcard: vcard.to_string(), phone, jid }
}

fn context(m: &wa::Message) -> Option<&wa::ContextInfo> {
    macro_rules! ctx {
        ($($field:ident),*) => {
            $(if let Some(x) = m.$field.as_option() { return x.context_info.as_option(); })*
        };
    }
    ctx!(
        extended_text_message, image_message, video_message, audio_message, document_message, sticker_message,
        location_message, contact_message, contacts_array_message, poll_creation_message, poll_creation_message_v2,
        poll_creation_message_v3, poll_creation_message_v5, poll_creation_message_v6, event_message, ptv_message
    );
    None
}

fn poll(p: &wa::message::PollCreationMessage) -> Content {
    Content::Poll {
        question: p.name.clone().unwrap_or_default(),
        options: p
            .options
            .iter()
            .map(|o| PollOption { name: o.option_name.clone().unwrap_or_default(), voters: vec![] })
            .collect(),
        selectable: p.selectable_options_count.unwrap_or(0),
    }
}

/// The visible content of a (base) message, if it has any.
pub fn content_of(m: &wa::Message) -> Option<Content> {
    if let Some(t) = &m.conversation {
        return Some(Content::Text { text: t.clone(), preview: None });
    }
    if let Some(x) = m.extended_text_message.as_option() {
        let url = x.matched_text.clone().unwrap_or_default();
        let preview = (!url.is_empty() && x.title.as_ref().is_some_and(|t| !t.is_empty())).then(|| LinkPreview {
            site: host_of(&url),
            url,
            title: x.title.clone().unwrap_or_default(),
            description: x.description.clone().unwrap_or_default(),
            thumb: thumb(&x.jpeg_thumbnail),
        });
        return Some(Content::Text { text: x.text.clone().unwrap_or_default(), preview });
    }
    if let Some(x) = m.image_message.as_option() {
        return Some(Content::Image {
            media: remote(&x.mimetype, x.file_length),
            caption: x.caption.clone().unwrap_or_default(),
            width: x.width.unwrap_or(0),
            height: x.height.unwrap_or(0),
            thumb: thumb(&x.jpeg_thumbnail),
        });
    }
    if let Some(x) = m.video_message.as_option().or(m.ptv_message.as_option()) {
        return Some(Content::Video {
            media: remote(&x.mimetype, x.file_length),
            caption: x.caption.clone().unwrap_or_default(),
            width: x.width.unwrap_or(0),
            height: x.height.unwrap_or(0),
            duration: x.seconds.unwrap_or(0),
            gif: x.gif_playback.unwrap_or(false),
            thumb: thumb(&x.jpeg_thumbnail),
        });
    }
    if let Some(x) = m.audio_message.as_option() {
        return Some(Content::Audio {
            media: remote(&x.mimetype, x.file_length),
            duration: x.seconds.unwrap_or(0),
            voice: x.ptt.unwrap_or(false),
            waveform: x.waveform.clone().unwrap_or_default().into_iter().map(|v| v.min(100)).collect(),
        });
    }
    if let Some(x) = m.document_message.as_option() {
        return Some(Content::Document {
            media: remote(&x.mimetype, x.file_length),
            name: x.file_name.clone().or_else(|| x.title.clone()).unwrap_or_else(|| "Document".into()),
            caption: x.caption.clone().unwrap_or_default(),
            pages: x.page_count.unwrap_or(0),
            thumb: thumb(&x.jpeg_thumbnail),
        });
    }
    if let Some(x) = m.sticker_message.as_option() {
        return Some(Content::Sticker {
            media: remote(&x.mimetype, x.file_length),
            width: x.width.unwrap_or(512),
            height: x.height.unwrap_or(512),
            animated: x.is_animated.unwrap_or(false),
        });
    }
    if let Some(x) = m.location_message.as_option() {
        return Some(Content::Location {
            lat: x.degrees_latitude.unwrap_or(0.0),
            lng: x.degrees_longitude.unwrap_or(0.0),
            name: x.name.clone().unwrap_or_default(),
            address: x.address.clone().unwrap_or_default(),
            live: x.is_live.unwrap_or(false),
            thumb: thumb(&x.jpeg_thumbnail),
        });
    }
    if let Some(x) = m.live_location_message.as_option() {
        return Some(Content::Location {
            lat: x.degrees_latitude.unwrap_or(0.0),
            lng: x.degrees_longitude.unwrap_or(0.0),
            name: x.caption.clone().unwrap_or_default(),
            address: String::new(),
            live: true,
            thumb: thumb(&x.jpeg_thumbnail),
        });
    }
    if let Some(x) = m.contact_message.as_option() {
        let card = vcard_contact(x.display_name.as_deref().unwrap_or("Contact"), x.vcard.as_deref().unwrap_or(""));
        return Some(Content::Contacts { cards: vec![card] });
    }
    if let Some(x) = m.contacts_array_message.as_option() {
        let cards = x
            .contacts
            .iter()
            .map(|c| vcard_contact(c.display_name.as_deref().unwrap_or("Contact"), c.vcard.as_deref().unwrap_or("")))
            .collect();
        return Some(Content::Contacts { cards });
    }
    for p in [
        &m.poll_creation_message,
        &m.poll_creation_message_v2,
        &m.poll_creation_message_v3,
        &m.poll_creation_message_v5,
        &m.poll_creation_message_v6,
    ] {
        if let Some(p) = p.as_option() {
            return Some(poll(p));
        }
    }
    if let Some(x) = m.event_message.as_option() {
        return Some(Content::Event {
            name: x.name.clone().unwrap_or_default(),
            description: x.description.clone().unwrap_or_default(),
            location: x
                .location
                .as_option()
                .map(|l| l.name.clone().or_else(|| l.address.clone()).unwrap_or_default())
                .unwrap_or_default(),
            start: x.start_time.unwrap_or(0) * 1000,
            end: x.end_time.unwrap_or(0) * 1000,
            canceled: x.is_canceled.unwrap_or(false),
            responses: EventResponses::default(),
        });
    }
    if let Some(x) = m.call_log_messsage.as_option() {
        use wa::message::call_log_message::CallOutcome as O;
        let missed = !matches!(x.call_outcome, Some(O::CONNECTED) | Some(O::ACCEPTED_ELSEWHERE) | Some(O::ONGOING));
        return Some(Content::Call {
            video: x.is_video.unwrap_or(false),
            missed,
            duration: x.duration_secs.unwrap_or(0).max(0) as u32,
        });
    }
    if let Some(x) = m.group_invite_message.as_option() {
        let name = x.group_name.clone().unwrap_or_else(|| "a group".into());
        let code = x.invite_code.clone().unwrap_or_default();
        return Some(Content::Text {
            text: format!("Invitation to join {name}\nhttps://chat.whatsapp.com/{code}"),
            preview: None,
        });
    }
    None
}

/// Interprets a decrypted message.
pub fn parse(msg: &wa::Message) -> Parsed {
    let mut out = Parsed {
        view_once: msg.is_view_once(),
        secret: msg.message_context_info.as_option().and_then(|c| c.message_secret.clone()).filter(|s| !s.is_empty()),
        ..Default::default()
    };
    let base = msg.get_base_message();

    if let Some(assoc) = msg.message_context_info.as_option().and_then(|c| c.message_association.as_option()) {
        use wa::message_association::AssociationType as A;
        if assoc.association_type == Some(A::MEDIA_ALBUM) {
            out.album = assoc.parent_message_key.as_option().and_then(|k| k.id.clone());
        }
    }

    // ── things that act on another message ──
    if let Some(r) = base.reaction_message.as_option()
        && let Some(id) = r.key.as_option().and_then(|k| k.id.clone()) {
            out.special = Special::Reaction { target: id, emoji: r.text.clone().unwrap_or_default() };
            return out;
        }
    if let Some(p) = base.protocol_message.as_option() {
        use wa::message::protocol_message::Type as T;
        let target = p.key.as_option().and_then(|k| k.id.clone()).unwrap_or_default();
        out.special = match p.r#type {
            Some(T::REVOKE) if !target.is_empty() => Special::Revoke { target },
            Some(T::MESSAGE_EDIT) if !target.is_empty() => {
                let edited = p.edited_message.as_option().map(|m| m.get_base_message());
                let text = edited
                    .and_then(|m| m.text_content().or(m.get_caption()))
                    .unwrap_or_default()
                    .to_string();
                Special::Edit { target, text }
            }
            Some(T::EPHEMERAL_SETTING) => Special::Ephemeral { secs: p.ephemeral_expiration.unwrap_or(0) },
            _ => Special::Skip,
        };
        return out;
    }
    if let Some(v) = base.poll_update_message.as_option() {
        if let (Some(id), Some(vote)) = (v.poll_creation_message_key.as_option().and_then(|k| k.id.clone()), v.vote.as_option()) {
            out.special = Special::PollVote {
                poll: id,
                payload: vote.enc_payload.clone().unwrap_or_default(),
                iv: vote.enc_iv.clone().unwrap_or_default(),
            };
        } else {
            out.special = Special::Skip;
        }
        return out;
    }
    if let Some(e) = base.enc_event_response_message.as_option() {
        out.special = match e.event_creation_message_key.as_option().and_then(|k| k.id.clone()) {
            Some(id) => Special::EventResponse {
                event: id,
                payload: e.enc_payload.clone().unwrap_or_default(),
                iv: e.enc_iv.clone().unwrap_or_default(),
            },
            None => Special::Skip,
        };
        return out;
    }
    if let Some(p) = base.pin_in_chat_message.as_option() {
        use wa::message::pin_in_chat_message::Type as T;
        out.special = match p.key.as_option().and_then(|k| k.id.clone()) {
            Some(id) => Special::Pin { target: id, on: p.r#type == Some(T::PIN_FOR_ALL) },
            None => Special::Skip,
        };
        return out;
    }
    if base.keep_in_chat_message.is_set() || base.album_message.is_set() {
        out.special = Special::Skip;
        return out;
    }

    // ── ordinary content ──
    out.content = content_of(base);
    if out.content.is_none() {
        // Key distribution and other transport-only payloads carry nothing to show.
        let transport_only = base.sender_key_distribution_message.is_set()
            || base.fast_ratchet_key_sender_key_distribution_message.is_set()
            || base.message_history_bundle.is_set()
            || base.message_history_notice.is_set();
        if transport_only || *base == wa::Message::default() {
            out.special = Special::Skip;
        } else {
            out.content = Some(Content::Unsupported { note: String::new() });
        }
        return out;
    }
    if let Some(ctx) = context(base) {
        out.mentions = ctx.mentioned_jid.iter().map(|j| bare(j)).collect();
        out.forwarded = if ctx.is_forwarded.unwrap_or(false) { ctx.forwarding_score.unwrap_or(1).max(1) } else { 0 };
        out.expiration = ctx.expiration.unwrap_or(0);
        if let (Some(id), Some(q)) = (&ctx.stanza_id, ctx.quoted_message.as_option()) {
            let quoted = content_of(q.get_base_message());
            out.quote = Some(Quote {
                id: id.clone(),
                sender: ctx.participant.as_deref().map(bare).unwrap_or_default(),
                text: quoted.as_ref().map(|c| truncate(&c.summary(), 240)).unwrap_or_default(),
                kind: quoted.as_ref().map(|c| c.kind().to_string()).unwrap_or_else(|| "text".into()),
                thumb: match &quoted {
                    Some(Content::Image { thumb, .. } | Content::Video { thumb, .. } | Content::Document { thumb, .. }) => {
                        thumb.clone()
                    }
                    _ => None,
                },
            });
        }
    }
    out
}

/// Where to fetch a message's media from, as stored next to the message.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Stored {
    /// The full protobuf message, base64 (needed to download, forward or quote).
    #[serde(default)]
    pub raw: String,
    /// Message secret, base64 (polls and events).
    #[serde(default)]
    pub secret: String,
    /// The sender's address exactly as it arrived (poll and event keys depend on it).
    #[serde(default)]
    pub sender: String,
    /// A newer CDN path for the media, after the phone uploaded it again.
    #[serde(default)]
    pub direct_path: String,
}

impl Stored {
    pub fn of(msg: &wa::Message, secret: Option<&[u8]>) -> Stored {
        Stored {
            raw: b64(&msg.encode_to_vec()),
            secret: secret.map(b64).unwrap_or_default(),
            sender: String::new(),
            direct_path: String::new(),
        }
    }

    pub fn message(&self) -> Option<wa::Message> {
        if self.raw.is_empty() {
            return None;
        }
        wa::Message::decode_from_slice(&unb64(&self.raw)).ok()
    }
}

/// A timestamp that may be in seconds or milliseconds, as milliseconds.
pub fn ms(ts: i64) -> i64 {
    if ts > 0 && ts < 100_000_000_000 { ts * 1000 } else { ts }
}

/// When a mute ends, in milliseconds: 0 for not muted, -1 for always.
/// History sync sends seconds and settings sync sends milliseconds; "always" arrives
/// as -1 or as its unsigned twin.
pub fn mute_until(raw: i64) -> i64 {
    if raw < 0 { -1 } else { ms(raw) }
}

fn remote_sticker(
    direct_path: &Option<String>,
    media_key: &Option<Vec<u8>>,
    file_enc_sha256: &Option<Vec<u8>>,
    file_length: Option<u64>,
    mimetype: &Option<String>,
) -> Option<RemoteSticker> {
    Some(RemoteSticker {
        direct_path: direct_path.clone().filter(|p| !p.is_empty())?,
        media_key: b64(media_key.as_ref().filter(|k| !k.is_empty())?),
        file_enc_sha256: b64(file_enc_sha256.as_ref().filter(|h| !h.is_empty())?),
        file_length: file_length.unwrap_or(0),
        mimetype: mimetype.clone().unwrap_or_else(|| "image/webp".into()),
    })
}

/// A library entry for a sticker: id (file hash), where it is stored, whether animated.
pub struct StickerIdent {
    pub id: String,
    pub remote: Option<RemoteSticker>,
    pub animated: bool,
}

/// The library identity of a sticker message. Lottie stickers are skipped:
/// Ripple can't draw them.
pub fn sticker_of_message(m: &wa::Message) -> Option<StickerIdent> {
    let x = m.get_base_message().sticker_message.as_option()?;
    if x.is_lottie.unwrap_or(false) {
        return None;
    }
    let hash = x.file_sha256.as_ref().filter(|h| !h.is_empty())?;
    Some(StickerIdent {
        id: b64(hash),
        remote: remote_sticker(&x.direct_path, &x.media_key, &x.file_enc_sha256, x.file_length, &x.mimetype),
        animated: x.is_animated.unwrap_or(false),
    })
}

/// The library identity of a recent sticker listed in history sync.
pub fn sticker_of_metadata(x: &wa::StickerMetadata) -> Option<StickerIdent> {
    if x.is_lottie.unwrap_or(false) {
        return None;
    }
    let hash = x.file_sha256.as_ref().filter(|h| !h.is_empty())?;
    Some(StickerIdent {
        id: b64(hash),
        remote: remote_sticker(&x.direct_path, &x.media_key, &x.file_enc_sha256, x.file_length, &x.mimetype),
        animated: false,
    })
}

/// Where a favourited sticker is stored, from a synced favourite action.
pub fn sticker_of_action(x: &wa::sync_action_value::StickerAction) -> Option<RemoteSticker> {
    remote_sticker(&x.direct_path, &x.media_key, &x.file_enc_sha256, x.file_length, &x.mimetype)
}

/// The favourite action WhatsApp syncs for a sticker.
pub fn favorite_action(s: &Sticker, r: &RemoteSticker, on: bool, now: i64) -> wa::SyncActionValue {
    let act = pb!(wa::sync_action_value::StickerAction {
        direct_path: Some(r.direct_path.clone()),
        media_key: Some(unb64(&r.media_key)),
        file_enc_sha256: Some(unb64(&r.file_enc_sha256)),
        file_length: Some(r.file_length),
        mimetype: Some(r.mimetype.clone()),
        width: Some(s.width),
        height: Some(s.height),
        is_favorite: Some(on),
        is_lottie: Some(false),
        is_avatar_sticker: Some(false)
    });
    pb!(wa::SyncActionValue { sticker_action: MessageField::some(act), timestamp: Some(now) })
}

/// The action that drops a sticker from "Recent" on every device.
pub fn unrecent_action(s: &Sticker, now: i64) -> wa::SyncActionValue {
    let act = pb!(wa::sync_action_value::RemoveRecentStickerAction { last_sticker_sent_ts: Some(s.used) });
    pb!(wa::SyncActionValue { remove_recent_sticker_action: MessageField::some(act), timestamp: Some(now) })
}

/// Whether a message is worth keeping in its raw form.
pub fn needs_raw(c: &Content) -> bool {
    c.media().is_some() || matches!(c, Content::Poll { .. } | Content::Event { .. })
}

// ───────────────────────────── outgoing ─────────────────────────────

/// Result of uploading a file, in the shape the media messages need.
pub struct Uploaded {
    pub url: String,
    pub direct_path: String,
    pub media_key: Vec<u8>,
    pub file_enc_sha256: Vec<u8>,
    pub file_sha256: Vec<u8>,
    pub file_length: u64,
    pub media_key_timestamp: i64,
}

fn context_for(m: &Message) -> Option<wa::ContextInfo> {
    if m.quote.is_none() && m.mentions.is_empty() && m.forwarded == 0 && m.expires == 0 {
        return None;
    }
    let mut ctx = wa::ContextInfo::default();
    if let Some(q) = &m.quote {
        ctx.stanza_id = Some(q.id.clone());
        if !q.sender.is_empty() {
            ctx.participant = Some(q.sender.clone());
        }
        ctx.quoted_message = MessageField::some(pb!(wa::Message { conversation: Some(q.text.clone()) }));
    }
    ctx.mentioned_jid = m.mentions.clone();
    if m.forwarded > 0 {
        ctx.is_forwarded = Some(true);
        ctx.forwarding_score = Some(m.forwarded);
    }
    Some(ctx)
}

/// Builds the protobuf for an outgoing message. `up` must be given for media.
pub fn build(m: &Message, up: Option<&Uploaded>, thumb_jpeg: Option<Vec<u8>>, ephemeral: u32) -> anyhow::Result<wa::Message> {
    let mut ctx = context_for(m);
    if ephemeral > 0 {
        ctx.get_or_insert_with(Default::default).expiration = Some(ephemeral);
    }
    let field = |c: &Option<wa::ContextInfo>| c.clone().map(MessageField::some).unwrap_or_default();
    let need = || up.ok_or_else(|| anyhow::anyhow!("media was not uploaded"));
    let mut msg = wa::Message::default();
    match &m.content {
        Content::Text { text, preview } => {
            if ctx.is_none() && preview.is_none() {
                msg.conversation = Some(text.clone());
            } else {
                let mut x = pb!(wa::message::ExtendedTextMessage { text: Some(text.clone()) });
                if let Some(p) = preview {
                    x.matched_text = Some(p.url.clone());
                    x.title = Some(p.title.clone());
                    x.description = Some(p.description.clone());
                    x.jpeg_thumbnail = p.thumb.as_deref().map(unb64);
                }
                x.context_info = field(&ctx);
                msg.extended_text_message = MessageField::some(x);
            }
        }
        Content::Image { media, caption, width, height, .. } => {
            let u = need()?;
            msg.image_message = MessageField::some(pb!(wa::message::ImageMessage {
                url: Some(u.url.clone()),
                direct_path: Some(u.direct_path.clone()),
                media_key: Some(u.media_key.clone()),
                file_enc_sha256: Some(u.file_enc_sha256.clone()),
                file_sha256: Some(u.file_sha256.clone()),
                file_length: Some(u.file_length),
                media_key_timestamp: Some(u.media_key_timestamp),
                mimetype: Some(if media.mime.is_empty() { "image/jpeg".into() } else { media.mime.clone() }),
                caption: (!caption.is_empty()).then(|| caption.clone()),
                width: Some(*width),
                height: Some(*height),
                jpeg_thumbnail: thumb_jpeg,
                view_once: m.view_once.then_some(true),
                context_info: field(&ctx) }));
        }
        Content::Video { media, caption, width, height, duration, gif, .. } => {
            let u = need()?;
            msg.video_message = MessageField::some(pb!(wa::message::VideoMessage {
                url: Some(u.url.clone()),
                direct_path: Some(u.direct_path.clone()),
                media_key: Some(u.media_key.clone()),
                file_enc_sha256: Some(u.file_enc_sha256.clone()),
                file_sha256: Some(u.file_sha256.clone()),
                file_length: Some(u.file_length),
                media_key_timestamp: Some(u.media_key_timestamp),
                mimetype: Some(if media.mime.is_empty() { "video/mp4".into() } else { media.mime.clone() }),
                caption: (!caption.is_empty()).then(|| caption.clone()),
                width: Some(*width),
                height: Some(*height),
                seconds: Some(*duration),
                gif_playback: gif.then_some(true),
                jpeg_thumbnail: thumb_jpeg,
                view_once: m.view_once.then_some(true),
                context_info: field(&ctx) }));
        }
        Content::Audio { media, duration, voice, waveform } => {
            let u = need()?;
            msg.audio_message = MessageField::some(pb!(wa::message::AudioMessage {
                url: Some(u.url.clone()),
                direct_path: Some(u.direct_path.clone()),
                media_key: Some(u.media_key.clone()),
                file_enc_sha256: Some(u.file_enc_sha256.clone()),
                file_sha256: Some(u.file_sha256.clone()),
                file_length: Some(u.file_length),
                media_key_timestamp: Some(u.media_key_timestamp),
                mimetype: Some(if media.mime.is_empty() { "audio/ogg; codecs=opus".into() } else { media.mime.clone() }),
                seconds: Some(*duration),
                ptt: Some(*voice),
                waveform: (!waveform.is_empty()).then(|| waveform.clone()),
                context_info: field(&ctx) }));
        }
        Content::Document { media, name, caption, .. } => {
            let u = need()?;
            msg.document_message = MessageField::some(pb!(wa::message::DocumentMessage {
                url: Some(u.url.clone()),
                direct_path: Some(u.direct_path.clone()),
                media_key: Some(u.media_key.clone()),
                file_enc_sha256: Some(u.file_enc_sha256.clone()),
                file_sha256: Some(u.file_sha256.clone()),
                file_length: Some(u.file_length),
                media_key_timestamp: Some(u.media_key_timestamp),
                mimetype: Some(if media.mime.is_empty() { "application/octet-stream".into() } else { media.mime.clone() }),
                file_name: Some(name.clone()),
                title: Some(name.clone()),
                caption: (!caption.is_empty()).then(|| caption.clone()),
                jpeg_thumbnail: thumb_jpeg,
                context_info: field(&ctx) }));
        }
        Content::Sticker { width, height, animated, .. } => {
            let u = need()?;
            msg.sticker_message = MessageField::some(pb!(wa::message::StickerMessage {
                url: Some(u.url.clone()),
                direct_path: Some(u.direct_path.clone()),
                media_key: Some(u.media_key.clone()),
                file_enc_sha256: Some(u.file_enc_sha256.clone()),
                file_sha256: Some(u.file_sha256.clone()),
                file_length: Some(u.file_length),
                media_key_timestamp: Some(u.media_key_timestamp),
                mimetype: Some("image/webp".into()),
                width: Some(*width),
                height: Some(*height),
                is_animated: Some(*animated),
                context_info: field(&ctx) }));
        }
        Content::Location { lat, lng, name, address, .. } => {
            msg.location_message = MessageField::some(pb!(wa::message::LocationMessage {
                degrees_latitude: Some(*lat),
                degrees_longitude: Some(*lng),
                name: (!name.is_empty()).then(|| name.clone()),
                address: (!address.is_empty()).then(|| address.clone()),
                context_info: field(&ctx) }));
        }
        Content::Contacts { cards } => {
            let one = |c: &ContactCard| pb!(wa::message::ContactMessage {
                display_name: Some(c.name.clone()),
                vcard: Some(c.vcard.clone()) });
            if let [only] = cards.as_slice() {
                let mut c = one(only);
                c.context_info = field(&ctx);
                msg.contact_message = MessageField::some(c);
            } else {
                msg.contacts_array_message = MessageField::some(pb!(wa::message::ContactsArrayMessage {
                    display_name: Some(format!("{} contacts", cards.len())),
                    contacts: cards.iter().map(one).collect(),
                    context_info: field(&ctx) }));
            }
        }
        other => anyhow::bail!("{} messages are sent through their own request", other.kind()),
    }
    Ok(msg)
}

/// Maps a stored delivery status from a history record.
pub fn status_of(s: Option<wa::web_message_info::Status>) -> Status {
    use wa::web_message_info::Status as S;
    match s {
        Some(S::ERROR) => Status::Failed,
        Some(S::PENDING) => Status::Pending,
        Some(S::SERVER_ACK) => Status::Sent,
        Some(S::DELIVERY_ACK) => Status::Delivered,
        Some(S::READ) => Status::Read,
        Some(S::PLAYED) => Status::Played,
        _ => Status::Delivered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_reply_with_mentions() {
        let mut ctx = wa::ContextInfo::default();
        ctx.stanza_id = Some("ABC".into());
        ctx.participant = Some("15550001111:3@s.whatsapp.net".into());
        ctx.quoted_message = MessageField::some(pb!(wa::Message { conversation: Some("original".into()) }));
        ctx.mentioned_jid = vec!["15550002222@s.whatsapp.net".into()];
        ctx.is_forwarded = Some(true);
        ctx.forwarding_score = Some(6);
        let msg = pb!(wa::Message {
            extended_text_message: MessageField::some(pb!(wa::message::ExtendedTextMessage {
                text: Some("hello @15550002222".into()),
                context_info: MessageField::some(ctx) })) });
        let p = parse(&msg);
        assert_eq!(p.content.as_ref().unwrap().text(), "hello @15550002222");
        let q = p.quote.unwrap();
        assert_eq!((q.id.as_str(), q.sender.as_str(), q.text.as_str()), ("ABC", "15550001111@s.whatsapp.net", "original"));
        assert_eq!(p.mentions, vec!["15550002222@s.whatsapp.net".to_string()]);
        assert_eq!(p.forwarded, 6);
        assert!(matches!(p.special, Special::None));
    }

    #[test]
    fn recognises_specials() {
        let key = pb!(wa::MessageKey { id: Some("TARGET".into()) });
        let reaction = pb!(wa::Message {
            reaction_message: MessageField::some(pb!(wa::message::ReactionMessage {
                key: MessageField::some(key.clone()),
                text: Some("🔥".into()) })) });
        assert!(matches!(parse(&reaction).special, Special::Reaction { ref target, ref emoji } if target == "TARGET" && emoji == "🔥"));

        let revoke = pb!(wa::Message {
            protocol_message: MessageField::some(pb!(wa::message::ProtocolMessage {
                key: MessageField::some(key.clone()),
                r#type: Some(wa::message::protocol_message::Type::REVOKE) })) });
        assert!(matches!(parse(&revoke).special, Special::Revoke { ref target } if target == "TARGET"));

        let edit = pb!(wa::Message {
            protocol_message: MessageField::some(pb!(wa::message::ProtocolMessage {
                key: MessageField::some(key),
                r#type: Some(wa::message::protocol_message::Type::MESSAGE_EDIT),
                edited_message: MessageField::some(pb!(wa::Message { conversation: Some("fixed".into()) })) })) });
        assert!(matches!(parse(&edit).special, Special::Edit { ref text, .. } if text == "fixed"));
        assert!(matches!(parse(&wa::Message::default()).special, Special::Skip));
    }

    #[test]
    fn media_round_trips_through_storage() {
        let msg = pb!(wa::Message {
            image_message: MessageField::some(pb!(wa::message::ImageMessage {
                mimetype: Some("image/jpeg".into()),
                caption: Some("view".into()),
                width: Some(640),
                height: Some(480),
                file_length: Some(1234),
                direct_path: Some("/v/t62/abc".into()) })) });
        let p = parse(&msg);
        let c = p.content.unwrap();
        assert_eq!(c.kind(), "image");
        assert_eq!(c.media().unwrap().size, 1234);
        assert!(needs_raw(&c));
        let stored = Stored::of(&msg, Some(&[1, 2, 3]));
        let back = stored.message().unwrap();
        assert_eq!(back.image_message.as_option().unwrap().direct_path.as_deref(), Some("/v/t62/abc"));
        assert_eq!(unb64(&stored.secret), vec![1, 2, 3]);
    }

    #[test]
    fn builds_outgoing_text_and_reply() {
        let mut m = Message {
            id: "X".into(),
            chat: "1@s.whatsapp.net".into(),
            seq: 0,
            sender: "2@s.whatsapp.net".into(),
            from_me: true,
            ts: 0,
            content: Content::Text { text: "plain".into(), preview: None },
            status: Status::Pending,
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
        assert_eq!(build(&m, None, None, 0).unwrap().conversation.as_deref(), Some("plain"));
        m.quote = Some(Quote { id: "Q".into(), sender: "1@s.whatsapp.net".into(), text: "earlier".into(), kind: "text".into(), thumb: None });
        let built = build(&m, None, None, 86_400).unwrap();
        let x = built.extended_text_message.as_option().unwrap();
        let ctx = x.context_info.as_option().unwrap();
        assert_eq!(ctx.stanza_id.as_deref(), Some("Q"));
        assert_eq!(ctx.expiration, Some(86_400));
        // Parsing what we built gives the same reply back.
        assert_eq!(parse(&built).quote.unwrap().text, "earlier");
        m.content = Content::Image { media: Media::default(), caption: String::new(), width: 1, height: 1, thumb: None };
        assert!(build(&m, None, None, 0).is_err(), "media needs an upload first");
    }

    #[test]
    fn sticker_identity() {
        let msg = pb!(wa::Message {
            sticker_message: MessageField::some(pb!(wa::message::StickerMessage {
                file_sha256: Some(vec![1; 32]),
                file_enc_sha256: Some(vec![2; 32]),
                media_key: Some(vec![3; 32]),
                direct_path: Some("/v/t62.15575-24/x".into()),
                file_length: Some(9000),
                is_animated: Some(true)
            }))
        });
        let s = sticker_of_message(&msg).unwrap();
        assert_eq!(s.id, b64(&[1; 32]));
        assert!(s.animated);
        let r = s.remote.unwrap();
        assert_eq!((r.direct_path.as_str(), r.file_length, r.mimetype.as_str()), ("/v/t62.15575-24/x", 9000, "image/webp"));
        // Round trip into the favourite action the phone expects.
        let lib = Sticker { id: s.id, width: 512, height: 512, used: 5, ..Default::default() };
        let v = favorite_action(&lib, &r, true, 77);
        let act = v.sticker_action.as_option().unwrap();
        assert_eq!(act.is_favorite, Some(true));
        assert_eq!(act.media_key.as_deref(), Some(&[3u8; 32][..]));
        assert_eq!(sticker_of_action(act).unwrap(), r);
        assert_eq!(unrecent_action(&lib, 77).remove_recent_sticker_action.as_option().unwrap().last_sticker_sent_ts, Some(5));
        assert_eq!(ms(1_700_000_000), 1_700_000_000_000);
        assert_eq!(ms(1_700_000_000_000), 1_700_000_000_000);
        // Lottie stickers are left out.
        let mut lottie = msg.clone();
        lottie.sticker_message.as_option_mut().unwrap().is_lottie = Some(true);
        assert!(sticker_of_message(&lottie).is_none());
    }

    #[test]
    fn vcard_parsing() {
        let c = vcard_contact("Ana", "BEGIN:VCARD\nVERSION:3.0\nFN:Ana\nTEL;type=CELL;waid=15550001111:+1 555 000 1111\nEND:VCARD");
        assert_eq!(c.jid.as_deref(), Some("15550001111@s.whatsapp.net"));
        assert_eq!(c.phone.as_deref(), Some("+1 555 000 1111"));
    }
}
