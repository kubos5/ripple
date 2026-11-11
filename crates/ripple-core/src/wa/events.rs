//! Handles everything the server tells us.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use whatsapp_rust::Jid as WaJid;
use whatsapp_rust::client::Client;
use whatsapp_rust::types::events::Event;
use whatsapp_rust::types::message::MessageInfo;
use whatsapp_rust::wacore::stanza::groups::{GroupNotificationAction as GA, GroupParticipantInfo};
use whatsapp_rust::wacore::types::call::CallAction;
use whatsapp_rust::wacore::types::events::BatchOrigin;
use whatsapp_rust::wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use whatsapp_rust::waproto::whatsapp as wa;

use super::convert::{self, Special, Stored, jid_str, parse_jid};
use super::{CallInfo, Inner, group_from, lock, merge_group};
use crate::app::App;
use crate::model::*;

fn is_chat(jid: &str) -> bool {
    jid.ends_with(USER_SERVER) || jid.ends_with("@g.us") || jid.ends_with("@lid")
}

fn blank(id: String, chat: String, sender: String, from_me: bool, ts: i64, content: Content) -> Message {
    Message {
        id,
        chat,
        seq: 0,
        sender,
        from_me,
        ts,
        content,
        status: if from_me { Status::Sent } else { Status::Delivered },
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

impl Inner {
    pub(crate) async fn on_event(self: &Arc<Self>, ev: Arc<Event>, client: Arc<Client>) {
        let Some(app) = self.app() else { return };
        match &*ev {
            // ── session ──
            Event::PairingQrCode(q) => app.set_session(Session::Pairing {
                qr: Some(q.code.clone()),
                code: None,
                expires: now_ms() + q.timeout.as_millis() as i64,
            }),
            Event::PairingCode(c) => app.set_session(Session::Pairing {
                qr: None,
                code: Some(c.code.clone()),
                expires: now_ms() + c.timeout.as_millis() as i64,
            }),
            Event::PairingQrCodesExhausted(_) | Event::PairingCodeError(_) | Event::PairError(_) => {
                let me = self.clone();
                let app = app.clone();
                tokio::spawn(async move {
                    me.stop().await;
                    me.forget_session(&app);
                    app.set_session(Session::Welcome);
                    app.toast("error", "Linking timed out or was refused. Please try again.");
                });
            }
            Event::PairSuccess(_) => {
                let _ = app.store.set_meta("paired", &true);
                self.first_sync.store(true, Ordering::Relaxed);
                super::mark_history(self);
                app.set_session(Session::Syncing { progress: 2 });
            }
            Event::Connected(_) => {
                let me = self.clone();
                tokio::spawn(async move { me.on_connected(app, client).await });
            }
            Event::Disconnected(_) => {
                if app.store.meta::<bool>("paired").unwrap_or(false) && !super::first_sync(self) {
                    app.set_session(Session::Offline);
                }
            }
            Event::StreamReplaced(_) => {
                app.set_session(Session::Offline);
                app.toast("error", "Ripple was opened somewhere else with this session.");
            }
            Event::ClientOutdated(_) => {
                app.toast("error", "WhatsApp no longer accepts this version of Ripple. Please update.");
                app.set_session(Session::Offline);
            }
            Event::TemporaryBan(_) => {
                app.toast("error", "WhatsApp has temporarily restricted this account.");
                app.set_session(Session::Offline);
            }
            Event::LoggedOut(l) => {
                let reason = l
                    .logout_message
                    .as_ref()
                    .and_then(|m| m.header.clone().or_else(|| m.subtext.clone()))
                    .unwrap_or_else(|| "This device was unlinked from your account.".into());
                let me = self.clone();
                tokio::spawn(async move {
                    me.stop().await;
                    me.forget_session(&app);
                    app.set_session(Session::LoggedOut { reason });
                });
            }
            Event::OfflineSyncCompleted(_) => {
                if !super::first_sync(self) {
                    app.set_session(Session::Online);
                }
            }

            // ── messages ──
            Event::Messages(batch) => {
                let live = matches!(batch.origin, BatchOrigin::Live);
                for m in batch.messages.iter() {
                    self.on_message(&app, &client, &m.message, &m.info, live && !m.info.is_offline).await;
                }
            }
            Event::Receipt(r) => {
                let status = match r.r#type {
                    ReceiptType::Delivered => Status::Delivered,
                    ReceiptType::Read | ReceiptType::ReadSelf => Status::Read,
                    ReceiptType::Played | ReceiptType::PlayedSelf => Status::Played,
                    _ => return,
                };
                let src = &r.source;
                let alt = if src.is_group { None } else { src.sender_alt.as_ref().or(src.recipient_alt.as_ref()) };
                let chat = self.resolve(&client, &src.chat, alt).await;
                if src.is_from_me {
                    // We read the chat on another device.
                    if matches!(r.r#type, ReceiptType::ReadSelf | ReceiptType::Read) && app.chat(&chat).is_some_and(|c| c.unread > 0) {
                        app.chat_mut(&chat, |c| {
                            c.unread = 0;
                            c.unread_mentions = 0;
                            c.marked_unread = false;
                        });
                    }
                    return;
                }
                let by = if src.is_group { Some(self.resolve(&client, &src.sender, src.sender_alt.as_ref()).await) } else { None };
                let ts = r.timestamp.timestamp_millis();
                for id in &r.message_ids {
                    // LID/PN drift can leave the receipt's chat id different from ours; fall back to the id alone.
                    let at = if app.store.message(&chat, id).is_some() {
                        chat.clone()
                    } else {
                        match app.store.message_by_id(id) {
                            Some(m) => m.chat,
                            None => continue,
                        }
                    };
                    app.ingest_status(&at, id, status, by.as_deref(), ts);
                }
            }
            Event::ChatPresence(p) => {
                let src = &p.source;
                let alt = if src.is_group { None } else { src.sender_alt.as_ref() };
                let chat = self.resolve(&client, &src.chat, alt).await;
                let user = self.resolve(&client, &src.sender, src.sender_alt.as_ref()).await;
                let state = match (&p.state, &p.media) {
                    (ChatPresence::Composing, ChatPresenceMedia::Audio) => "recording",
                    (ChatPresence::Composing, _) => "typing",
                    _ => "idle",
                };
                app.ingest_typing(&chat, &user, state);
            }
            Event::Presence(p) => {
                let jid = self.resolve(&client, &p.from, None).await;
                app.ingest_presence(&jid, !p.unavailable, p.last_seen.map(|t| t.timestamp_millis()).unwrap_or(0));
            }

            // ── people ──
            Event::PushNameUpdate(p) => {
                let jid = self.resolve(&client, &p.jid, p.message.source.sender_alt.as_ref()).await;
                let name = p.new_push_name.clone();
                if !name.is_empty() {
                    app.ingest_contact(&jid, |c| c.push_name = Some(name));
                }
            }
            Event::ContactUpdate(c) => {
                let a = &c.action;
                if let (Some(lid), Some(pn)) = (&a.lid_jid, &a.pn_jid) {
                    self.learn(&bare(lid), &bare(pn));
                }
                let jid = self.resolve(&client, &c.jid, None).await;
                let name = a.full_name.clone().or_else(|| a.first_name.clone()).filter(|n| !n.is_empty());
                if c.from_full_sync {
                    app.store.upsert_contact(&jid, |x| x.name = name);
                } else {
                    app.ingest_contact(&jid, |x| x.name = name);
                }
            }
            Event::UserAboutUpdate(u) => {
                let jid = self.resolve(&client, &u.jid, None).await;
                let about = u.status.clone();
                app.ingest_contact(&jid, |c| c.about = Some(about));
            }
            Event::PictureUpdate(p) => {
                let id = self.resolve(&client, &p.jid, None).await;
                if p.removed {
                    self.set_avatar(&app, &id, None);
                } else {
                    let me = self.clone();
                    tokio::spawn(async move { me.fetch_avatar(&app, &client, &id, true).await });
                }
            }

            // ── groups ──
            Event::GroupUpdate(u) => self.on_group_update(&app, &client, u).await,

            // ── chat state synced from our other devices ──
            Event::PinUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                let on = u.action.pinned.unwrap_or(false);
                let ts = u.timestamp.timestamp_millis();
                app.chat_mut(&id, |c| c.pinned = if on { ts.max(1) } else { 0 });
            }
            Event::MuteUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                let until = if u.action.muted.unwrap_or(false) { u.action.mute_end_timestamp.unwrap_or(-1) } else { 0 };
                app.chat_mut(&id, |c| c.muted_until = until);
            }
            Event::ArchiveUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                let on = u.action.archived.unwrap_or(false);
                app.chat_mut(&id, |c| c.archived = on);
            }
            Event::MarkChatAsReadUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                let read = u.action.read.unwrap_or(true);
                app.chat_mut(&id, |c| {
                    c.marked_unread = !read;
                    if read {
                        c.unread = 0;
                        c.unread_mentions = 0;
                    }
                });
            }
            Event::DeleteChatUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                if app.chat(&id).is_some() {
                    app.remove_chat(&id);
                }
            }
            Event::ClearChatUpdate(u) => {
                let id = self.resolve(&client, &u.jid, None).await;
                if app.chat(&id).is_some() {
                    let _ = app.store.clear_chat(&id);
                    app.chat_mut(&id, |c| {
                        c.last = None;
                        c.unread = 0;
                    });
                    app.emit(crate::api::UiEvent::History { chat: id });
                }
            }
            Event::StarUpdate(u) => {
                let chat = self.resolve(&client, &u.chat_jid, None).await;
                app.ingest_star(&chat, &u.message_id, u.action.starred.unwrap_or(false));
            }
            Event::DeleteMessageForMeUpdate(u) => {
                let chat = self.resolve(&client, &u.chat_jid, None).await;
                if app.store.message(&chat, &u.message_id).is_some() {
                    app.ingest_deleted_for_me(&chat, &u.message_id);
                }
            }

            // ── history ──
            Event::HistorySync(_) => {
                let me = self.clone();
                let app2 = app.clone();
                let ev2 = ev.clone();
                // Decompressing and decoding a history chunk is CPU work; keep it off the async threads.
                let done = tokio::task::spawn_blocking(move || {
                    if let Event::HistorySync(h) = &*ev2 { me.on_history(&app2, h) } else { None }
                })
                .await;
                super::mark_history(self);
                if let (Ok(Some(progress)), true) = (done, super::first_sync(self)) {
                    app.set_session(Session::Syncing { progress: progress.clamp(2, 99) as u8 });
                }
            }

            // ── calls ──
            Event::IncomingCall(c) => self.on_call(&app, &client, c).await,
            Event::MissedCall(m) => {
                let from = self.resolve(&client, &m.from, None).await;
                lock(&self.calls).remove(&m.call_id);
                app.ingest_call(&m.call_id, &from, false, false, true);
            }
            Event::CallEndedElsewhere(_) => {
                let ids: Vec<(String, WaJid)> = lock(&self.calls).drain().map(|(id, c)| (id, c.from)).collect();
                for (id, from) in ids {
                    app.ingest_call(&id, &jid_str(&from), false, false, false);
                }
            }
            _ => {}
        }
    }

    // ───────────────────────────── connection ─────────────────────────────

    async fn on_connected(self: Arc<Self>, app: Arc<App>, client: Arc<Client>) {
        let _ = app.store.set_meta("paired", &true);
        if let Some(pn) = client.pn() {
            let me = jid_str(&pn);
            if let Some(lid) = client.lid() {
                self.learn(&jid_str(&lid), &me);
            }
            let name = client.push_name();
            app.set_me(&me, (!name.is_empty()).then_some(name));
        }
        if super::first_sync(&self) {
            app.set_session(Session::Syncing { progress: 5 });
            // History arrives in chunks; call it done once they stop coming.
            let (me, app2) = (self.clone(), app.clone());
            tokio::spawn(async move {
                let started = now_ms();
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let quiet = now_ms() - me.last_history.load(Ordering::Relaxed);
                    if quiet > 12_000 || now_ms() - started > 180_000 {
                        break;
                    }
                }
                me.first_sync.store(false, Ordering::Relaxed);
                app2.set_session(Session::Online);
                // Bulk-loaded names and chats: have the UI reload its lists once.
                app2.emit(crate::api::UiEvent::Reset);
            });
        } else {
            app.set_session(Session::Online);
        }
        let _ = client.presence().set_available().await;

        self.refresh_groups(&app, &client).await;

        if let Ok(res) = client.fetch_privacy_settings().await {
            use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory as C, PrivacyValue as V};
            let mut p = app.store.meta::<Privacy>("privacy").unwrap_or_default();
            for s in res.settings {
                let v = match &s.value {
                    V::All => "all",
                    V::Contacts | V::ContactBlacklist => "contacts",
                    V::None => "none",
                    V::MatchLastSeen => "match_last_seen",
                    _ => "all",
                }
                .to_string();
                match s.category {
                    C::Last => p.last_seen = v,
                    C::Online => p.online = v,
                    C::Profile => p.profile_photo = v,
                    C::Status => p.about = v,
                    C::GroupAdd => p.group_add = v,
                    C::ReadReceipts => p.read_receipts = !matches!(s.value, V::None),
                    _ => {}
                }
            }
            app.set_privacy(p);
        }
        if let Ok(list) = client.blocking().get_blocklist().await {
            let blocked: std::collections::HashSet<String> = list.iter().map(|b| self.canon(&b.jid, None)).collect();
            for c in app.store.contacts() {
                if c.blocked != blocked.contains(&c.id) {
                    let on = blocked.contains(&c.id);
                    app.ingest_contact(&c.id, |x| x.blocked = on);
                }
            }
            for id in blocked {
                if app.store.contact(&id).is_none() {
                    app.ingest_contact(&id, |x| x.blocked = true);
                }
            }
        }

        // Anything that was still waiting to go out when we last lost the connection.
        for m in app.store.pending(now_ms() - 86_400_000) {
            let id = m.id.clone();
            let chat = m.chat.clone();
            match self.send(&app, &client, &m).await {
                Ok(()) => {
                    app.message_mut(&chat, &id, |x| {
                        if x.status == Status::Pending {
                            x.status = Status::Sent;
                        }
                    });
                }
                Err(e) => log::warn!("resend of {id} failed: {e:#}"),
            }
        }
        self.clone().fill_avatars(app, client);
    }

    pub(crate) async fn refresh_groups(&self, app: &Arc<App>, client: &Arc<Client>) {
        let me = app.me();
        let Ok(groups) = client.groups().get_participating().await else { return };
        let mut communities = vec![];
        for md in groups.values() {
            let g = group_from(self, &me, md);
            if g.is_community {
                communities.push(md.id.clone());
            }
            merge_group(app, g);
        }
        for parent in communities {
            let pid = jid_str(&parent);
            let Ok(subs) = client.community().get_subgroups(&parent).await else { continue };
            for s in subs {
                let id = jid_str(&s.id);
                if app.store.group(&id).is_some() {
                    app.group_mut(&id, |g| {
                        g.parent = Some(pid.clone());
                        g.is_announcement = s.is_default_sub_group;
                        if let Some(n) = s.participant_count {
                            g.size = n;
                        }
                    });
                } else {
                    app.ingest_group(Group {
                        id,
                        subject: s.subject.clone(),
                        owner: s.owner.as_ref().map(|o| self.canon(o, None)),
                        created_at: s.creation.unwrap_or(0) as i64 * 1000,
                        parent: Some(pid.clone()),
                        is_announcement: s.is_default_sub_group,
                        joined: false,
                        size: s.participant_count.unwrap_or(0),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // ───────────────────────────── one message ─────────────────────────────

    async fn on_message(self: &Arc<Self>, app: &Arc<App>, client: &Arc<Client>, msg: &wa::Message, info: &MessageInfo, live: bool) {
        let src = &info.source;
        let raw_chat = jid_str(&src.chat);
        if !is_chat(&raw_chat) {
            return; // status updates, channels, broadcasts
        }
        let me = app.me();
        let chat_alt = if src.is_group {
            None
        } else if src.is_from_me {
            src.recipient_alt.as_ref()
        } else {
            src.sender_alt.as_ref()
        };
        let chat = self.resolve(client, &src.chat, chat_alt).await;
        let sender = if src.is_from_me { me.clone() } else { self.resolve(client, &src.sender, src.sender_alt.as_ref()).await };
        let ts = info.timestamp.timestamp_millis();
        let parsed = convert::parse(msg);

        if !src.is_from_me && !info.push_name.is_empty() {
            let known = app.store.contact(&sender).and_then(|c| c.push_name);
            if known.as_deref() != Some(info.push_name.as_str()) {
                let name = info.push_name.clone();
                app.ingest_contact(&sender, |c| c.push_name = Some(name));
            }
        }

        match parsed.special {
            Special::Skip => return,
            Special::Reaction { target, emoji } => return app.ingest_reaction(&chat, &target, &sender, &emoji, ts),
            Special::Revoke { target } => return app.ingest_revoke(&chat, &target),
            Special::Edit { target, text } => return app.ingest_edit(&chat, &target, &text, ts),
            Special::Pin { target, on } => {
                let secs = msg.message_context_info.as_option().and_then(|c| c.message_add_on_duration_in_secs).unwrap_or(7 * 86_400);
                return app.ingest_pin(&chat, &target, if on { ts + secs as i64 * 1000 } else { 0 }, &sender);
            }
            Special::Ephemeral { secs } => return app.ingest_ephemeral(&chat, secs, &sender),
            Special::PollVote { poll, payload, iv } => {
                return self.on_poll_vote(app, &chat, &poll, &sender, &jid_str(&src.sender), src.sender_alt.as_ref().map(jid_str), &payload, &iv);
            }
            Special::EventResponse { event, payload, iv } => {
                return self.on_event_response(app, &chat, &event, &sender, &jid_str(&src.sender), src.sender_alt.as_ref().map(jid_str), &payload, &iv);
            }
            Special::None => {}
        }
        let Some(content) = parsed.content else { return };
        let mut m = blank(info.id.clone(), chat.clone(), sender, src.is_from_me, ts, content);
        m.quote = parsed.quote.map(|mut q| {
            q.sender = self.canon_s(&q.sender);
            q
        });
        m.mentions = parsed.mentions.iter().map(|j| self.canon_s(j)).collect();
        m.forwarded = parsed.forwarded;
        m.view_once = parsed.view_once;
        m.album = parsed.album;
        let timer = info.ephemeral_expiration.unwrap_or(parsed.expiration);
        if timer > 0 {
            m.expires = ts + timer as i64 * 1000;
        }
        if convert::needs_raw(&m.content) {
            let mut stored = Stored::of(msg, parsed.secret.as_deref());
            stored.sender = jid_str(&src.sender);
            let _ = app.store.set_media_params(&chat, &m.id, &stored);
        }
        if let Some(c) = app.chat(&chat)
            && c.ephemeral != timer && timer > 0 && c.kind == ChatKind::Direct {
                app.chat_mut(&chat, |c| c.ephemeral = timer);
            }
        if live { app.ingest_message(m, true) } else { app.ingest_offline(m) };
    }

    #[allow(clippy::too_many_arguments)]
    fn on_poll_vote(&self, app: &App, chat: &str, poll: &str, voter: &str, voter_raw: &str, voter_alt: Option<String>, payload: &[u8], iv: &[u8]) {
        use whatsapp_rust::wacore::poll::{PollVoteCiphertext, compute_option_hash, decrypt_poll_vote_with_secret};
        let Some(stored) = app.store.media_params::<Stored>(chat, poll) else { return };
        let Some(target) = app.store.message(chat, poll) else { return };
        let Content::Poll { options, .. } = &target.content else { return };
        let secret = convert::unb64(&stored.secret);
        if secret.is_empty() {
            return;
        }
        // The key derivation uses the addresses exactly as the voter's device saw them, so try each form.
        let lid_of = |pn: &str| lock(&self.pns).get(pn).cloned();
        let mut creators = vec![stored.sender.clone(), target.sender.clone()];
        creators.extend(lid_of(&target.sender));
        let mut voters = vec![voter_raw.to_string(), voter.to_string()];
        voters.extend(voter_alt);
        voters.extend(lid_of(voter));
        for creator in creators.iter().filter(|c| !c.is_empty()) {
            for v in &voters {
                let Ok(hashes) = decrypt_poll_vote_with_secret(PollVoteCiphertext { enc_payload: payload, enc_iv: iv }, &secret, poll, creator, v) else { continue };
                let chosen: Vec<String> = options
                    .iter()
                    .filter(|o| hashes.iter().any(|h| h.as_slice() == compute_option_hash(&o.name).as_slice()))
                    .map(|o| o.name.clone())
                    .collect();
                return app.ingest_poll_vote(chat, poll, voter, &chosen);
            }
        }
        log::debug!("could not decrypt a poll vote for {poll}");
    }

    #[allow(clippy::too_many_arguments)]
    fn on_event_response(&self, app: &App, chat: &str, event: &str, who: &str, who_raw: &str, who_alt: Option<String>, payload: &[u8], iv: &[u8]) {
        use wa::message::event_response_message::EventResponseType as R;
        use whatsapp_rust::wacore::event::decrypt_event_response_with_secret;
        let Some(stored) = app.store.media_params::<Stored>(chat, event) else { return };
        let Some(target) = app.store.message(chat, event) else { return };
        let secret = convert::unb64(&stored.secret);
        if secret.is_empty() {
            return;
        }
        let lid_of = |pn: &str| lock(&self.pns).get(pn).cloned();
        let mut creators = vec![stored.sender.clone(), target.sender.clone()];
        creators.extend(lid_of(&target.sender));
        let mut responders = vec![who_raw.to_string(), who.to_string()];
        responders.extend(who_alt);
        responders.extend(lid_of(who));
        for creator in creators.iter().filter(|c| !c.is_empty()) {
            for r in &responders {
                let Ok(res) = decrypt_event_response_with_secret(payload, iv, &secret, event, creator, r) else { continue };
                let answer = match res.response {
                    Some(R::GOING) => "going",
                    Some(R::MAYBE) => "maybe",
                    Some(R::NOT_GOING) => "declined",
                    _ => "none",
                };
                return app.ingest_rsvp(chat, event, who, answer);
            }
        }
    }

    // ───────────────────────────── history ─────────────────────────────

    /// Stores one history chunk. Returns the sync progress (0–100) if the chunk reported one.
    fn on_history(&self, app: &Arc<App>, h: &whatsapp_rust::wacore::types::events::LazyHistorySync) -> Option<u32> {
        let hs = h.get()?;
        let me = app.me();
        for map in &hs.phone_number_to_lid_mappings {
            if let (Some(pn), Some(lid)) = (&map.pn_jid, &map.lid_jid) {
                self.learn(&bare(lid), &bare(pn));
            }
        }
        let bulk = super::first_sync(self);
        for p in &hs.pushnames {
            let (Some(id), Some(name)) = (&p.id, &p.pushname) else { continue };
            if name.is_empty() {
                continue;
            }
            let id = self.canon_s(id);
            if bulk {
                app.store.upsert_contact(&id, |c| c.push_name = Some(name.clone()));
            } else {
                app.ingest_contact(&id, |c| c.push_name = Some(name.clone()));
            }
        }
        let mut batch: Vec<Message> = Vec::new();
        let mut states = Vec::new();
        for conv in &hs.conversations {
            if !is_chat(&conv.id) {
                continue;
            }
            if let (Some(pn), Some(lid)) = (&conv.pn_jid, &conv.lid_jid) {
                self.learn(&bare(lid), &bare(pn));
            } else if let (true, Some(pn)) = (conv.id.ends_with("@lid"), &conv.pn_jid) {
                self.learn(&bare(&conv.id), &bare(pn));
            }
            let chat = self.canon_s(&conv.id);
            if !is_group(&chat)
                && let Some(name) = conv.display_name.clone().or_else(|| conv.name.clone()).filter(|n| !n.is_empty()) {
                    app.store.upsert_contact(&chat, |c| {
                        if c.name.is_none() && c.push_name.is_none() {
                            c.push_name = Some(name);
                        }
                    });
                }
            for item in &conv.messages {
                let Some(w) = item.message.as_option() else { continue };
                if let Some(m) = self.from_history(app, w, &chat, &me) {
                    batch.push(m);
                }
            }
            states.push((chat, conv));
        }
        app.ingest_history(batch);
        for (chat, conv) in states {
            if app.chat(&chat).is_none() {
                if conv.messages.is_empty() && !is_group(&chat) {
                    continue;
                }
                app.ensure_chat(&chat);
            }
            let stamp = conv.last_msg_timestamp.or(conv.conversation_timestamp).unwrap_or(0) as i64 * 1000;
            app.chat_mut(&chat, |c| {
                if let Some(n) = conv.unread_count {
                    c.unread = n;
                }
                if let Some(n) = conv.unread_mention_count {
                    c.unread_mentions = n;
                }
                c.marked_unread = conv.marked_as_unread.unwrap_or(c.marked_unread);
                c.archived = conv.archived.unwrap_or(c.archived);
                if let Some(p) = conv.pinned {
                    c.pinned = p as i64 * 1000;
                }
                if let Some(mute) = conv.mute_end_time {
                    c.muted_until = if mute == 0 { 0 } else { mute as i64 };
                }
                if let Some(e) = conv.ephemeral_expiration {
                    c.ephemeral = e;
                }
                if c.last.is_none() && stamp > 0 {
                    c.ts = stamp;
                }
            });
        }
        hs.progress
    }

    /// Converts one stored message from a history chunk.
    fn from_history(&self, app: &App, w: &wa::WebMessageInfo, chat: &str, me: &str) -> Option<Message> {
        let key = w.key.as_option()?;
        let id = key.id.clone()?;
        let msg = w.message.as_option()?;
        let from_me = key.from_me.unwrap_or(false);
        let sender = if from_me {
            me.to_string()
        } else if is_group(chat) {
            self.canon_s(key.participant.as_deref().or(w.participant.as_deref())?)
        } else {
            chat.to_string()
        };
        let ts = w.message_timestamp.unwrap_or(0) as i64 * 1000;
        let parsed = convert::parse(msg);
        if !matches!(parsed.special, Special::None) {
            return None;
        }
        let mut m = blank(id, chat.to_string(), sender, from_me, ts, parsed.content?);
        m.status = if from_me { convert::status_of(w.status) } else { Status::Delivered };
        m.quote = parsed.quote.map(|mut q| {
            q.sender = self.canon_s(&q.sender);
            q
        });
        m.mentions = parsed.mentions.iter().map(|j| self.canon_s(j)).collect();
        m.forwarded = parsed.forwarded;
        m.view_once = parsed.view_once;
        m.album = parsed.album;
        m.starred = w.starred.unwrap_or(false);
        if let Some(until) = w.ephemeral_expiration_timestamp.filter(|t| *t > 0) {
            m.expires = until as i64 * 1000;
        } else if let (Some(start), Some(d)) = (w.ephemeral_start_timestamp, w.ephemeral_duration)
            && d > 0 {
                m.expires = (start as i64 + d as i64) * 1000;
            }
        for r in &w.reactions {
            let Some(text) = r.text.clone().filter(|t| !t.is_empty()) else { continue };
            let by = match r.key.as_option() {
                Some(k) if k.from_me.unwrap_or(false) => me.to_string(),
                Some(k) => match &k.participant {
                    Some(p) => self.canon_s(p),
                    None => chat.to_string(),
                },
                None => continue,
            };
            m.reactions.push(Reaction { emoji: text, sender: by, ts: r.sender_timestamp_ms.unwrap_or(ts) });
        }
        if from_me {
            for r in &w.user_receipt {
                let user = self.canon_s(&r.user_jid);
                for (stamp, status) in [(r.receipt_timestamp, Status::Delivered), (r.read_timestamp, Status::Read), (r.played_timestamp, Status::Played)] {
                    if let Some(t) = stamp.filter(|t| *t > 0) {
                        let _ = app.store.put_receipt(chat, &m.id, &user, status, t * 1000);
                    }
                }
            }
        }
        if convert::needs_raw(&m.content) {
            let secret = w.message_secret.clone().or(parsed.secret);
            let mut stored = Stored::of(msg, secret.as_deref());
            stored.sender = key.participant.clone().unwrap_or_default();
            let _ = app.store.set_media_params(chat, &m.id, &stored);
        }
        Some(m)
    }

    // ───────────────────────────── groups ─────────────────────────────

    async fn on_group_update(&self, app: &Arc<App>, client: &Arc<Client>, u: &whatsapp_rust::wacore::types::events::GroupUpdate) {
        let id = jid_str(&u.group_jid);
        let me = app.me();
        if app.store.group(&id).is_none()
            && let Ok(md) = client.groups().get_metadata(&u.group_jid).await {
                merge_group(app, group_from(self, &me, &md));
            }
        let actor = match &u.participant {
            Some(p) => Some(self.resolve(client, p, u.participant_pn.as_ref()).await),
            None => None,
        };
        let who = |list: &[GroupParticipantInfo]| -> Vec<String> {
            list.iter().map(|p| self.canon(&p.jid, p.phone_number.as_ref().or(p.lid.as_ref()))).collect()
        };
        let sys = |event: &str, targets: &[String], text: &str| app.system_message(&id, event, actor.as_deref(), targets, text);
        match &u.action {
            GA::Add { participants, .. } => {
                let jids = who(participants);
                app.group_mut(&id, |g| {
                    for j in &jids {
                        if g.role_of(j).is_none() {
                            g.participants.push(Participant { id: j.clone(), role: Role::Member });
                        }
                    }
                    if jids.contains(&me) {
                        g.joined = true;
                    }
                });
                if actor.as_ref().is_some_and(|a| jids.len() == 1 && jids[0] == *a) || actor.is_none() {
                    app.system_message(&id, "join", jids.first().map(|s| s.as_str()), &[], "");
                } else {
                    sys("add", &jids, "");
                }
            }
            GA::Remove { participants, .. } => {
                let jids = who(participants);
                app.group_mut(&id, |g| {
                    g.participants.retain(|p| !jids.contains(&p.id));
                    if jids.contains(&me) {
                        g.joined = false;
                    }
                });
                if actor.as_ref().is_none_or(|a| jids.len() == 1 && jids[0] == *a) {
                    app.system_message(&id, "leave", jids.first().map(|s| s.as_str()), &[], "");
                } else {
                    sys("remove", &jids, "");
                }
            }
            GA::Promote { participants } | GA::Demote { participants } => {
                let up = matches!(u.action, GA::Promote { .. });
                let jids = who(participants);
                app.group_mut(&id, |g| {
                    for p in g.participants.iter_mut().filter(|p| jids.contains(&p.id) && p.role != Role::Owner) {
                        p.role = if up { Role::Admin } else { Role::Member };
                    }
                });
                sys(if up { "promote" } else { "demote" }, &jids, "");
            }
            GA::Subject { subject, .. } => {
                let s = subject.clone();
                app.group_mut(&id, |g| g.subject = s);
                sys("subject", &[], subject);
            }
            GA::Description { description, .. } => {
                let d = description.clone().filter(|d| !d.is_empty());
                app.group_mut(&id, |g| g.description = d);
                sys("description", &[], "");
            }
            GA::Locked { .. } | GA::Unlocked => {
                let on = matches!(u.action, GA::Locked { .. });
                app.group_mut(&id, |g| g.locked = on);
                sys("locked", &[], if on { "on" } else { "off" });
            }
            GA::Announce | GA::NotAnnounce => {
                let on = matches!(u.action, GA::Announce);
                app.group_mut(&id, |g| g.announce = on);
                sys("announce", &[], if on { "on" } else { "off" });
            }
            GA::Ephemeral { expiration, .. } => app.ingest_ephemeral(&id, *expiration, actor.as_deref().unwrap_or_default()),
            GA::MembershipApprovalMode { enabled } => {
                let on = *enabled;
                app.group_mut(&id, |g| g.approval = on);
                sys("approval", &[], if on { "on" } else { "off" });
            }
            GA::CreatedMembershipRequests { requests, .. } => {
                let jids = who(requests);
                app.group_mut(&id, |g| {
                    for j in jids {
                        if !g.pending.contains(&j) {
                            g.pending.push(j);
                        }
                    }
                });
            }
            GA::RevokedMembershipRequests { participants } => {
                let jids: Vec<String> = participants.iter().map(|p| self.canon(p, None)).collect();
                app.group_mut(&id, |g| g.pending.retain(|j| !jids.contains(j)));
            }
            GA::MemberAddMode { mode } => {
                let on = mode == "all_member_add";
                app.group_mut(&id, |g| g.members_can_add = on);
                sys("membersCanAdd", &[], if on { "on" } else { "off" });
            }
            GA::Invite { code } => {
                let c = code.clone();
                app.group_mut(&id, |g| g.invite_code = Some(c));
            }
            GA::RevokeInvite => {
                app.group_mut(&id, |g| g.invite_code = None);
            }
            GA::Create { .. } => {
                if let Ok(md) = client.groups().get_metadata(&u.group_jid).await {
                    let g = group_from(self, &me, &md);
                    let subject = g.subject.clone();
                    merge_group(app, g);
                    sys("create", &[], &subject);
                }
            }
            _ => {}
        }
    }

    // ───────────────────────────── calls ─────────────────────────────

    async fn on_call(&self, app: &Arc<App>, client: &Arc<Client>, c: &whatsapp_rust::wacore::types::call::IncomingCall) {
        match &c.action {
            CallAction::Offer { call_id, call_creator, caller_pn, is_video, .. } => {
                let from = self.resolve(client, call_creator, caller_pn.as_ref()).await;
                if c.offline {
                    return app.ingest_call(call_id, &from, *is_video, false, true);
                }
                lock(&self.calls).insert(
                    call_id.clone(),
                    CallInfo { from: c.from.clone(), creator: call_creator.clone(), video: *is_video, answered: false },
                );
                app.ingest_call(call_id, &from, *is_video, true, false);
            }
            CallAction::Accept { call_id, .. } => {
                if let Some(info) = lock(&self.calls).get_mut(call_id) {
                    info.answered = true;
                }
            }
            CallAction::Terminate { call_id, call_creator, .. } | CallAction::Reject { call_id, call_creator, .. } => {
                let Some(info) = lock(&self.calls).remove(call_id) else { return };
                let from = self.resolve(client, call_creator, None).await;
                app.ingest_call(call_id, &from, info.video, false, !info.answered);
            }
            _ => {}
        }
    }

    // ───────────────────────────── avatars ─────────────────────────────

    pub(crate) fn set_avatar(&self, app: &App, id: &str, token: Option<String>) {
        let old = if is_group(id) { app.store.group(id).and_then(|g| g.avatar) } else { app.store.contact(id).and_then(|c| c.avatar) };
        if old == token {
            return;
        }
        if is_group(id) {
            let t = token.clone();
            app.group_mut(id, |g| g.avatar = t);
            app.chat_mut(id, |c| c.avatar = token);
        } else {
            app.ingest_contact(id, |c| c.avatar = token);
        }
        if let Some(path) = old.and_then(|t| app.media.path(&t)) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Downloads a profile picture (`small` = thumbnail) and attaches it.
    pub(crate) async fn fetch_avatar(&self, app: &Arc<App>, client: &Arc<Client>, id: &str, small: bool) {
        let Ok(jid) = parse_jid(id) else { return };
        let pic = match client.contacts().get_profile_picture(&jid, small).await {
            Ok(Some(p)) if !p.url.is_empty() => p,
            _ => return,
        };
        let bytes = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            let agent: ureq::Agent =
                ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
            agent.get(&pic.url).call().ok()?.body_mut().with_config().limit(4_000_000).read_to_vec().ok()
        })
        .await
        .ok()
        .flatten();
        let Some(bytes) = bytes.filter(|b| b.len() > 100) else { return };
        if let Ok(token) = app.media.write("a", "jpg", &bytes) {
            self.set_avatar(app, id, Some(token));
        }
    }

    /// Fetches pictures for the most recent chats that have none, gently.
    fn fill_avatars(self: Arc<Self>, app: Arc<App>, client: Arc<Client>) {
        if super::avatars_busy(&self).swap(true, Ordering::Relaxed) {
            return;
        }
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let mut chats = app.all_chats();
            chats.sort_by_key(|c| -c.ts);
            for c in chats.into_iter().filter(|c| c.avatar.is_none()).take(120) {
                if !client.is_connected() {
                    break;
                }
                self.fetch_avatar(&app, &client, &c.id, true).await;
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
            super::avatars_busy(&self).store(false, Ordering::Relaxed);
        });
    }
}
