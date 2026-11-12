//! A self-contained stand-in for WhatsApp: seeded conversations plus people
//! who read, type and answer. Lets the whole app be explored (and tested)
//! without linking an account.

mod art;
pub mod seed;

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::api::*;
use crate::app::{App, new_message_id};
use crate::media;
use crate::model::*;
use seed::*;

#[derive(Default)]
pub struct DemoNet {
    app: OnceLock<Weak<App>>,
    ringing: Arc<Mutex<Option<String>>>,
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn pick<'a>(list: &[&'a str]) -> &'a str {
    list[rand::random_range(0..list.len())]
}

fn incoming(chat: &str, from: &str, content: Content) -> Message {
    Message {
        id: new_message_id(),
        chat: chat.to_string(),
        seq: 0,
        sender: from.to_string(),
        from_me: false,
        ts: now_ms(),
        content,
        status: Status::Delivered,
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
    }
}

fn text(s: &str) -> Content {
    Content::Text { text: s.to_string(), preview: None }
}

const ANSWERS: [&str; 8] = [
    "Yes, definitely",
    "Hmm, let me check and get back to you",
    "I think so? Ask me again tomorrow 😄",
    "Works for me",
    "Not sure yet, depends on Friday",
    "Good question. No idea.",
    "Yes! Was about to ask you the same thing",
    "Probably, I'll confirm tonight",
];
const REPLIES: [&str; 10] = [
    "Ha, exactly",
    "Sounds good 👌",
    "Okay, noted",
    "Love that",
    "On my way to tell everyone",
    "That's the plan then",
    "😂",
    "Fair enough",
    "Perfect, thank you",
    "Agreed. Let's do it",
];
const MEDIA_REPLIES: [&str; 5] = ["Oh that's lovely", "Wow 😍", "Saving this one", "Where is that?", "Great shot"];

impl DemoNet {
    pub fn new() -> Self {
        Self::default()
    }

    fn app(&self) -> Result<Arc<App>> {
        self.app.get().and_then(|w| w.upgrade()).ok_or_else(|| anyhow!("The demo has ended"))
    }

    /// What the other side does after we send something: deliver, read, maybe react, answer.
    fn after_send(&self, app: &Arc<App>, m: &Message) {
        let weak = Arc::downgrade(app);
        let m = m.clone();
        tokio::spawn(async move {
            let Some(app) = weak.upgrade() else { return };
            let chat = m.chat.clone();
            if chat == ME {
                sleep(ms(300)).await;
                app.ingest_status(&chat, &m.id, Status::Read, None, now_ms());
                return;
            }
            let group = app.store.group(&chat);
            let others: Vec<String> = match &group {
                Some(g) => g.participants.iter().map(|p| p.id.clone()).filter(|j| j != ME).collect(),
                None => vec![chat.clone()],
            };
            if others.is_empty() {
                return;
            }
            sleep(ms(550)).await;
            for who in &others {
                let by = group.as_ref().map(|_| who.as_str());
                app.ingest_status(&chat, &m.id, Status::Delivered, by, now_ms());
            }
            sleep(ms(1100)).await;
            for who in &others {
                let by = group.as_ref().map(|_| who.as_str());
                app.ingest_status(&chat, &m.id, Status::Read, by, now_ms());
                if group.is_some() {
                    sleep(ms(120)).await;
                }
            }
            if matches!(m.content, Content::Audio { voice: true, .. }) && group.is_none() {
                sleep(ms(1800)).await;
                app.ingest_status(&chat, &m.id, Status::Played, None, now_ms());
            }
            let who = others[rand::random_range(0..others.len())].clone();
            if rand::random_range(0..100) < 35 {
                sleep(ms(500)).await;
                app.ingest_reaction(&chat, &m.id, &who, pick(&["❤️", "👍", "😂", "🔥", "🙏"]), now_ms());
            }
            if let Content::Poll { options, .. } = &m.content {
                for (i, voter) in others.iter().take(4).enumerate() {
                    sleep(ms(700)).await;
                    let choice = options[(i + rand::random_range(0..2)) % options.len()].name.clone();
                    app.ingest_poll_vote(&chat, &m.id, voter, &[choice]);
                }
                return;
            }
            if let Content::Event { .. } = &m.content {
                for (i, voter) in others.iter().take(3).enumerate() {
                    sleep(ms(800)).await;
                    app.ingest_rsvp(&chat, &m.id, voter, if i == 2 { "maybe" } else { "going" });
                }
                return;
            }
            if rand::random_range(0..100) < 20 {
                return;
            }
            sleep(ms(600)).await;
            app.ingest_typing(&chat, &who, "typing");
            sleep(ms(1500 + rand::random_range(0..1400))).await;
            app.ingest_typing(&chat, &who, "idle");
            let line = match &m.content {
                Content::Text { text, .. } if text.trim_end().ends_with('?') => pick(&ANSWERS),
                Content::Image { .. } | Content::Video { .. } => pick(&MEDIA_REPLIES),
                Content::Audio { .. } => "Just listened, call you in a bit",
                Content::Sticker { .. } => "😄",
                Content::Location { .. } => "Got it, see you there",
                _ => pick(&REPLIES),
            };
            let mut reply = incoming(&chat, &who, text(line));
            if rand::random_range(0..100) < 30 {
                reply.quote = Some(m.quote());
            }
            app.ingest_message(reply, true);
        });
    }

    fn ambient(&self, app: &Arc<App>) {
        let weak = Arc::downgrade(app);
        tokio::spawn(async move {
            sleep(ms(9_000)).await;
            let Some(app) = weak.upgrade() else { return };
            app.ingest_typing(LEO, LEO, "typing");
            sleep(ms(2_600)).await;
            app.ingest_typing(LEO, LEO, "idle");
            app.ingest_message(incoming(LEO, LEO, text("Okay sent! Check your email 📬")), true);
            drop(app);
            sleep(ms(19_000)).await;
            let Some(app) = weak.upgrade() else { return };
            app.ingest_typing(HIKERS, NOOR, "typing");
            sleep(ms(2_200)).await;
            app.ingest_typing(HIKERS, NOOR, "idle");
            app.ingest_message(incoming(HIKERS, NOOR, text("Just checked: the north car park opens at 7, so 7:45 is fine")), true);
        });
    }

    async fn trigger(&self, app: &Arc<App>, what: &str) -> Result<Value> {
        match what {
            "call" | "videocall" => {
                let id = media::random_hex(8).to_uppercase();
                *self.ringing.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
                let video = what == "videocall";
                app.ingest_call(&id, MAYA, video, true, false);
                let (weak, ringing) = (Arc::downgrade(app), self.ringing.clone());
                tokio::spawn(async move {
                    sleep(ms(14_000)).await;
                    let still = ringing.lock().unwrap_or_else(|e| e.into_inner()).take_if(|r| *r == id).is_some();
                    if let (true, Some(app)) = (still, weak.upgrade()) {
                        app.ingest_call(&id, MAYA, video, false, true);
                    }
                });
            }
            "message" => {
                let (chat, who, line) = match rand::random_range(0..3) {
                    0 => (DAD, DAD, "Did you see the match last night?"),
                    1 => (FLAT, CHLOE, "Parcel for you by the door 📦"),
                    _ => (AIKO, AIKO, "Sending you the photos tonight!"),
                };
                app.ingest_message(incoming(chat, who, text(line)), true);
            }
            "typing" => {
                let weak = Arc::downgrade(app);
                tokio::spawn(async move {
                    if let Some(app) = weak.upgrade() {
                        app.ingest_typing(MAYA, MAYA, "typing");
                    }
                    sleep(ms(4_000)).await;
                    if let Some(app) = weak.upgrade() {
                        app.ingest_typing(MAYA, MAYA, "idle");
                    }
                });
            }
            "pairing" | "pairingcode" => {
                let weak = Arc::downgrade(app);
                let code = what == "pairingcode";
                tokio::spawn(async move {
                    let Some(app) = weak.upgrade() else { return };
                    for round in 0..2 {
                        app.set_session(Session::Pairing {
                            qr: (!code).then(|| format!("2@RippleDemoPairing{round}{},demoNoiseKey,demoIdentityKey,demoAdvSecret", media::random_hex(24))),
                            code: code.then(|| "RPLD-EM0X".to_string()),
                            expires: now_ms() + 6_000,
                        });
                        sleep(ms(6_000)).await;
                    }
                    for p in [5u8, 20, 45, 70, 90, 100] {
                        app.set_session(Session::Syncing { progress: p });
                        sleep(ms(700)).await;
                    }
                    app.set_session(Session::Online);
                });
            }
            "offline" => {
                let weak = Arc::downgrade(app);
                tokio::spawn(async move {
                    let Some(app) = weak.upgrade() else { return };
                    app.set_session(Session::Offline);
                    sleep(ms(5_000)).await;
                    app.set_session(Session::Connecting);
                    sleep(ms(1_500)).await;
                    app.set_session(Session::Online);
                });
            }
            other => bail!("Unknown demo trigger: {other}"),
        }
        Ok(Value::Null)
    }

    async fn download(&self, app: &Arc<App>, target: &Message) -> Result<Value> {
        sleep(ms(900)).await;
        let seed = target.id.bytes().map(|b| b as u32).sum::<u32>();
        let kind = target.content.kind().to_string();
        let (w, h) = match &target.content {
            Content::Image { width, height, .. } => (*width, *height),
            _ => (0, 0),
        };
        let media_dir = app.clone();
        let token = tokio::task::spawn_blocking(move || -> Result<String> {
            let m = &media_dir.media;
            Ok(match kind.as_str() {
                "image" => {
                    let scale = 1100.0 / w.max(h).max(1) as f32;
                    let (w, h) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
                    m.write("m", "jpg", &art::jpeg(&art::scene(seed, w.max(64), h.max(64)), 84))?
                }
                "video" => m.write("m", "mp4", include_bytes!("assets/clip.mp4"))?,
                "gif" => m.write("m", "mp4", include_bytes!("assets/loop.mp4"))?,
                "sticker" => m.write("s", "webp", &art::sticker(seed))?,
                "document" => m.write("m", "pdf", &art::pdf("Demo document"))?,
                #[cfg(feature = "voice")]
                "voice" | "audio" => m.write("m", "ogg", &crate::voice::encode_ogg_opus(&art::speech_pcm(seed, 9.0))?)?,
                _ => bail!("This file isn't available in the demo"),
            })
        })
        .await??;
        Ok(json!({ "token": token }))
    }

    fn group_op(&self, app: &Arc<App>, op: &GroupOp) -> Result<Value> {
        let sys = |chat: &str, event: &str, targets: &[Jid], text: &str| {
            app.system_message(chat, event, Some(ME), targets, text);
        };
        Ok(match op {
            GroupOp::Create { subject, .. } => {
                let id = format!("1203639{}@g.us", rand::random_range(10_000_000_000u64..99_999_999_999));
                let (weak, chat, subject) = (Arc::downgrade(app), id.clone(), subject.clone());
                tokio::spawn(async move {
                    sleep(ms(80)).await;
                    if let Some(app) = weak.upgrade() {
                        app.system_message(&chat, "create", Some(ME), &[], &subject);
                    }
                });
                json!({ "id": id })
            }
            GroupOp::Refresh { .. } => Value::Null,
            GroupOp::Subject { id, text } => {
                sys(id, "subject", &[], text);
                Value::Null
            }
            GroupOp::Description { id, .. } => {
                sys(id, "description", &[], "");
                Value::Null
            }
            GroupOp::Setting { id, key, on } => {
                sys(id, key, &[], if *on { "on" } else { "off" });
                Value::Null
            }
            GroupOp::Participants { id, action, jids } => {
                sys(id, action, jids, "");
                Value::Null
            }
            GroupOp::Invite { .. } => json!({ "code": format!("{}Demo", media::random_hex(9)) }),
            GroupOp::InviteInfo { code } => {
                if code.trim().len() < 8 {
                    bail!("That invite link isn't valid");
                }
                json!({ "id": "120363999000000001@g.us", "subject": "Sunday Cyclists 🚴", "size": 23, "description": "Easy-paced social rides, coffee stop guaranteed." })
            }
            GroupOp::Join { code } => {
                if code.trim().len() < 8 {
                    bail!("That invite link isn't valid");
                }
                let id = "120363999000000001@g.us";
                app.ingest_group(Group {
                    id: id.into(),
                    subject: "Sunday Cyclists 🚴".into(),
                    description: Some("Easy-paced social rides, coffee stop guaranteed.".into()),
                    owner: Some(JONAS.into()),
                    created_at: now_ms() - 40 * 86_400_000,
                    participants: [(JONAS, Role::Owner), (LEO, Role::Member), (CHLOE, Role::Member), (ME, Role::Member)]
                        .iter()
                        .map(|(j, r)| Participant { id: j.to_string(), role: *r })
                        .collect(),
                    members_can_add: true,
                    joined: true,
                    ..Default::default()
                });
                sys(id, "join", &[], "");
                json!({ "id": id })
            }
            GroupOp::Leave { id } => {
                sys(id, "leave", &[], "");
                Value::Null
            }
            GroupOp::RequestAction { id, jids, approve } => {
                if *approve {
                    sys(id, "add", jids, "");
                }
                Value::Null
            }
            GroupOp::Icon { id, .. } => {
                sys(id, "icon", &[], "");
                Value::Null
            }
        })
    }
}

#[async_trait]
impl Network for DemoNet {
    async fn start(&self, app: Arc<App>) -> Result<()> {
        let _ = self.app.set(Arc::downgrade(&app));
        app.set_session(Session::Connecting);
        if app.all_chats().is_empty() {
            let a = app.clone();
            tokio::task::spawn_blocking(move || seed::seed(&a)).await?;
        }
        app.set_session(Session::Online);
        if std::env::var("RIPPLE_DEMO_QUIET").is_err() {
            self.ambient(&app);
        }
        Ok(())
    }

    async fn exec(&self, op: NetOp) -> Result<Value> {
        let app = self.app()?;
        match op {
            NetOp::Send { msg } => {
                sleep(ms(380)).await;
                self.after_send(&app, &msg);
                Ok(Value::Null)
            }
            NetOp::Download { target } => self.download(&app, &target).await,
            NetOp::Subscribe { jid } => {
                let online = [MAYA, AIKO, NOOR].contains(&jid.as_str());
                let seen = if online { 0 } else { now_ms() - (17 + jid.len() as i64 * 13 % 400) * 60_000 };
                app.ingest_presence(&jid, online, seen);
                Ok(Value::Null)
            }
            NetOp::CheckNumber { phone } => {
                let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
                sleep(ms(500)).await;
                Ok(json!({ "exists": digits.len() >= 8, "jid": format!("{digits}@s.whatsapp.net") }))
            }
            NetOp::Group(op) => {
                sleep(ms(250)).await;
                self.group_op(&app, &op)
            }
            NetOp::Community(op) => {
                sleep(ms(250)).await;
                Ok(match op {
                    CommunityOp::Create { .. } => {
                        let n = rand::random_range(10_000_000_000u64..99_999_999_999);
                        json!({ "id": format!("1203638{n}@g.us"), "announcement": format!("1203637{n}@g.us") })
                    }
                    _ => Value::Null,
                })
            }
            NetOp::RejectCall { .. } => {
                *self.ringing.lock().unwrap_or_else(|e| e.into_inner()) = None;
                Ok(Value::Null)
            }
            NetOp::DemoTrigger { what } => self.trigger(&app, &what).await,
            NetOp::Pair { .. } => bail!("Leave the demo to link a real account"),
            // What WhatsApp attaches to a sticker message, so the details view has something to show.
            NetOp::StickerInfo { .. } => Ok(json!({ "label": "A friendly cartoon sticker", "emojis": "😊" })),
            NetOp::Vote { .. } | NetOp::Rsvp { .. } | NetOp::Block { .. } | NetOp::Ephemeral { .. } => {
                sleep(ms(150)).await;
                Ok(Value::Null)
            }
            // Everything else just succeeds: the app has already applied it locally.
            _ => Ok(Value::Null),
        }
    }

    fn is_live(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ripple;

    fn temp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ripple-demo-test-{}", media::random_hex(5)))
    }

    async fn call(r: &Ripple, json: Value) -> Value {
        r.handle(serde_json::from_value(json).unwrap()).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn demo_boots_with_every_message_kind() {
        // SAFETY: tests in this module only ever set this variable.
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let init = call(&r, json!({ "cmd": "init" })).await;
        assert_eq!(init["session"]["state"], "online");
        assert_eq!(init["demo"], true);
        assert_eq!(init["me"]["id"], ME);
        let chats = init["chats"].as_array().unwrap();
        assert!(chats.len() >= 15, "only {} chats", chats.len());
        assert!(init["groups"].as_array().unwrap().iter().any(|g| g["isCommunity"] == true));

        let app = r.app();
        let mut kinds = std::collections::BTreeSet::new();
        for c in app.all_chats() {
            for m in app.store.messages(&c.id, None, 400) {
                kinds.insert(m.content.kind());
            }
        }
        for k in ["text", "image", "video", "gif", "voice", "document", "sticker", "location", "contacts", "poll", "event", "system", "call", "deleted"] {
            assert!(kinds.contains(k), "demo is missing a {k} message");
        }
        assert!(!app.store.search("ridge", None, 20).is_empty());
        assert_eq!(app.store.pinned(HIKERS, now_ms()).len(), 1);
        assert!(app.store.stickers().len() >= 5);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sending_goes_through_the_tick_states_and_gets_read() {
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let mut events = r.subscribe();
        let sent = call(&r, json!({ "cmd": "send", "chat": MAYA, "content": { "type": "text", "text": "Testing, testing?" } })).await;
        let id = sent["id"].as_str().unwrap().to_string();
        assert_eq!(sent["status"], "pending");

        let mut seen = vec![];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        while tokio::time::Instant::now() < deadline {
            let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await else { continue };
            if let UiEvent::Message { message, .. } = ev
                && message.id == id && seen.last() != Some(&message.status) {
                    seen.push(message.status);
                    if message.status == Status::Read {
                        break;
                    }
                }
        }
        assert_eq!(seen, vec![Status::Pending, Status::Sent, Status::Delivered, Status::Read]);
        let chat = r.app().chat(MAYA).unwrap();
        assert_eq!(chat.unread, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn message_actions_round_trip() {
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let app = r.app();
        let sent = call(&r, json!({ "cmd": "send", "chat": ME, "content": { "type": "text", "text": "note one" } })).await;
        let id = sent["id"].as_str().unwrap();

        call(&r, json!({ "cmd": "react", "chat": ME, "id": id, "emoji": "🔥" })).await;
        assert_eq!(app.store.message(ME, id).unwrap().reactions[0].emoji, "🔥");
        call(&r, json!({ "cmd": "react", "chat": ME, "id": id, "emoji": "🔥" })).await;
        assert!(app.store.message(ME, id).unwrap().reactions.is_empty(), "same emoji toggles off");

        call(&r, json!({ "cmd": "edit", "chat": ME, "id": id, "text": "note one, edited" })).await;
        let m = app.store.message(ME, id).unwrap();
        assert_eq!(m.content.text(), "note one, edited");
        assert!(m.edited > 0);
        assert_eq!(app.chat(ME).unwrap().last.unwrap().text, "note one, edited");

        call(&r, json!({ "cmd": "star", "chat": ME, "ids": [id], "on": true })).await;
        call(&r, json!({ "cmd": "pinMessage", "chat": ME, "id": id, "secs": 86400 })).await;
        assert_eq!(call(&r, json!({ "cmd": "pinned", "chat": ME })).await.as_array().unwrap().len(), 1);
        assert!(call(&r, json!({ "cmd": "starred" })).await.as_array().unwrap().iter().any(|m| m["id"] == id));

        call(&r, json!({ "cmd": "forward", "chat": ME, "ids": [id], "to": [DAD] })).await;
        let fwd = app.store.last_message(DAD).unwrap();
        assert_eq!(fwd.forwarded, 1);
        assert_eq!(fwd.content.text(), "note one, edited");
        assert!(app.chat(DAD).unwrap().draft.is_none(), "sending clears the draft");

        call(&r, json!({ "cmd": "delete", "chat": ME, "ids": [id], "everyone": true })).await;
        assert_eq!(app.store.message(ME, id).unwrap().content.kind(), "deleted");
        call(&r, json!({ "cmd": "delete", "chat": ME, "ids": [id], "everyone": false })).await;
        assert!(app.store.message(ME, id).is_none());
        assert_ne!(app.chat(ME).unwrap().last.unwrap().id, id, "preview falls back to the previous message");

        let poll = call(&r, json!({ "cmd": "send", "chat": ME, "content": { "type": "poll", "question": "Tea?", "options": ["Yes", "No"], "selectable": 1 } })).await;
        let pid = poll["id"].as_str().unwrap();
        call(&r, json!({ "cmd": "vote", "chat": ME, "id": pid, "options": ["Yes"] })).await;
        call(&r, json!({ "cmd": "vote", "chat": ME, "id": pid, "options": ["No"] })).await;
        match app.store.message(ME, pid).unwrap().content {
            Content::Poll { options, .. } => {
                assert!(options[0].voters.is_empty());
                assert_eq!(options[1].voters, vec![ME.to_string()]);
            }
            other => panic!("expected a poll, got {}", other.kind()),
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chat_and_group_management() {
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let app = r.app();

        let opened = call(&r, json!({ "cmd": "openChat", "chat": LEO })).await;
        assert_eq!(opened["unread"], 3);
        assert_eq!(app.chat(LEO).unwrap().unread, 0, "opening marks the chat read");

        call(&r, json!({ "cmd": "archive", "chat": LEO, "on": true })).await;
        call(&r, json!({ "cmd": "mute", "chat": LEO, "until": -1 })).await;
        let c = app.chat(LEO).unwrap();
        assert!(c.archived && c.muted_until == -1);

        // The demo starts with two pinned chats; a third is fine, a fourth is refused.
        call(&r, json!({ "cmd": "pinChat", "chat": DAD, "on": true })).await;
        let fourth = r.handle(serde_json::from_value(json!({ "cmd": "pinChat", "chat": PRIYA, "on": true })).unwrap()).await;
        assert!(fourth.is_err(), "only three chats can be pinned");
        call(&r, json!({ "cmd": "pinChat", "chat": DAD, "on": false })).await;
        assert_eq!(app.chat(DAD).unwrap().pinned, 0);

        let made = call(&r, json!({ "cmd": "group", "op": { "op": "create", "subject": "Climbing", "participants": [MAYA, NOOR] } })).await;
        let gid = made["id"].as_str().unwrap().to_string();
        let g = app.store.group(&gid).unwrap();
        assert_eq!(g.participants.len(), 3);
        assert_eq!(g.role_of(ME), Some(Role::Owner));
        assert_eq!(app.chat(&gid).unwrap().name, "Climbing");

        call(&r, json!({ "cmd": "group", "op": { "op": "participants", "id": gid, "action": "promote", "jids": [MAYA] } })).await;
        call(&r, json!({ "cmd": "group", "op": { "op": "participants", "id": gid, "action": "remove", "jids": [NOOR] } })).await;
        call(&r, json!({ "cmd": "group", "op": { "op": "subject", "id": gid, "text": "Climbing Crew" } })).await;
        call(&r, json!({ "cmd": "group", "op": { "op": "setting", "id": gid, "key": "announce", "on": true } })).await;
        let g = app.store.group(&gid).unwrap();
        assert_eq!(g.role_of(MAYA), Some(Role::Admin));
        assert!(g.role_of(NOOR).is_none() && g.announce);
        assert_eq!(app.chat(&gid).unwrap().name, "Climbing Crew");
        assert!(!app.chat(&gid).unwrap().read_only, "the owner can still post in an announce-only group");

        call(&r, json!({ "cmd": "group", "op": { "op": "leave", "id": gid } })).await;
        assert!(app.chat(&gid).unwrap().read_only);
        let blocked = r.handle(serde_json::from_value(json!({ "cmd": "send", "chat": gid, "content": { "type": "text", "text": "hi" } })).unwrap()).await;
        assert!(blocked.is_err());

        let com = call(&r, json!({ "cmd": "community", "op": { "op": "create", "name": "Allotments", "description": "Plots and plans" } })).await;
        let cid = com["id"].as_str().unwrap().to_string();
        assert!(app.store.group(&cid).unwrap().is_community);
        let ann = app.store.groups().into_iter().find(|g| g.parent.as_deref() == Some(cid.as_str()) && g.is_announcement).unwrap();
        assert_eq!(app.chat(&ann.id).unwrap().kind, ChatKind::Announcement);
        call(&r, json!({ "cmd": "community", "op": { "op": "link", "id": cid, "groups": [BOOKS] } })).await;
        assert_eq!(app.chat(BOOKS).unwrap().community.as_deref(), Some(cid.as_str()));
        call(&r, json!({ "cmd": "community", "op": { "op": "deactivate", "id": cid } })).await;
        assert!(app.store.group(&cid).is_none() && app.chat(&ann.id).is_none());
        assert!(app.chat(BOOKS).unwrap().community.is_none());

        call(&r, json!({ "cmd": "community", "op": { "op": "joinSubgroup", "id": MAPLE, "group": MAPLE_SWAP } })).await;
        assert!(app.store.group(MAPLE_SWAP).unwrap().joined && app.chat(MAPLE_SWAP).is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn media_download_and_incoming_unread() {
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let app = r.app();
        let remote = app
            .store
            .messages(DAD, None, 50)
            .into_iter()
            .find(|m| m.content.media().is_some_and(|x| x.state == MediaState::Remote))
            .unwrap();
        call(&r, json!({ "cmd": "download", "chat": DAD, "id": remote.id })).await;
        let mut ready = None;
        for _ in 0..40 {
            sleep(ms(150)).await;
            let m = app.store.message(DAD, &remote.id).unwrap();
            if m.content.media().unwrap().state == MediaState::Ready {
                ready = m.content.media().unwrap().token.clone();
                break;
            }
        }
        let token = ready.expect("download finished");
        assert!(r.media_path(&token).is_some());

        // A new message in a chat we are not looking at counts as unread and raises a notification.
        let mut events = r.subscribe();
        let before = app.chat(DAD).unwrap().unread;
        app.ingest_message(incoming(DAD, DAD, text("Ping")), true);
        assert_eq!(app.chat(DAD).unwrap().unread, before + 1);
        let mut notified = false;
        while let Ok(ev) = events.try_recv() {
            notified |= matches!(ev, UiEvent::Notify { ref chat, .. } if chat == DAD);
        }
        assert!(notified);

        // ...but not while that chat is open and the window focused.
        call(&r, json!({ "cmd": "openChat", "chat": DAD })).await;
        app.ingest_message(incoming(DAD, DAD, text("Pong")), true);
        assert_eq!(app.chat(DAD).unwrap().unread, 0);

        // Muted chats stay quiet, even when they mention us.
        let mut events = r.subscribe();
        app.ingest_message(incoming(FLAT, CHLOE, text("quiet")), true);
        let mut mention = incoming(FLAT, CHLOE, text("@you"));
        mention.mentions = vec![app.me()];
        app.ingest_message(mention, true);
        let mut noisy = false;
        while let Ok(ev) = events.try_recv() {
            noisy |= matches!(ev, UiEvent::Notify { .. });
        }
        assert!(!noisy);

        // Notifications name the sender from the contact book, even for a chat
        // that was created while only the number was known.
        let stranger = "15550100777@s.whatsapp.net";
        app.ensure_chat(stranger);
        assert_eq!(app.chat(stranger).unwrap().name, phone_of(stranger));
        app.store.upsert_contact(stranger, |c| c.name = Some("Rosa Lind".into()));
        let mut events = r.subscribe();
        app.ingest_message(incoming(stranger, stranger, text("Hello!")), true);
        let mut title = None;
        while let Ok(ev) = events.try_recv() {
            if let UiEvent::Notify { title: t, .. } = ev {
                title = Some(t);
            }
        }
        assert_eq!(title.as_deref(), Some("Rosa Lind"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sticker_details_and_save() {
        unsafe { std::env::set_var("RIPPLE_DEMO_QUIET", "1") };
        let dir = temp();
        let r = Ripple::open(dir.clone(), true).await.unwrap();
        let app = r.app();
        let m = app
            .store
            .sticker_messages(50)
            .into_iter()
            .find(|m| m.content.media().is_some_and(|x| x.state == MediaState::Ready))
            .expect("the demo has a downloaded sticker");
        let facts = call(&r, json!({ "cmd": "stickerDetails", "chat": m.chat, "id": m.id })).await;
        assert!(facts["id"].is_string() && facts["size"].as_u64().unwrap_or(0) > 0);
        assert_eq!(facts["label"], "A friendly cartoon sticker");
        assert_eq!(facts["emojis"], json!(["😊"]));

        let png = dir.join("saved.png");
        call(&r, json!({ "cmd": "stickerSave", "chat": m.chat, "id": m.id, "to": png.to_string_lossy() })).await;
        let bytes = std::fs::read(&png).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");

        // The library still lists a favourite whose file is gone, flagged so the picker can say so.
        let list: Vec<Sticker> = serde_json::from_value(call(&r, json!({ "cmd": "stickers" })).await).unwrap();
        assert!(list.iter().any(|s| s.favorite && s.missing && s.token.is_none()));
        std::fs::remove_dir_all(dir).ok();
    }
}
