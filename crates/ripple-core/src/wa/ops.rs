//! Carries out what the app asks of the network.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use whatsapp_rust::Jid as WaJid;
use whatsapp_rust::client::Client;
use whatsapp_rust::download::MediaType;
use whatsapp_rust::send::{PinDuration, RevokeType, SendOptions};
use whatsapp_rust::wacore::download::Downloadable;
use whatsapp_rust::wacore::proto_helpers::MessageExt;
use whatsapp_rust::waproto::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

use super::convert::{self, Stored, Uploaded, jid_str, parse_jid};
use super::{Inner, err, group_from, lock, merge_group};
use crate::api::*;
use crate::app::App;
use crate::model::*;

fn random_secret() -> Vec<u8> {
    (0..32).map(|_| rand::random::<u8>()).collect()
}

fn media_type(c: &Content) -> MediaType {
    match c {
        Content::Image { .. } => MediaType::Image,
        Content::Video { .. } => MediaType::Video,
        Content::Audio { .. } => MediaType::Audio,
        Content::Sticker { .. } => MediaType::Sticker,
        _ => MediaType::Document,
    }
}

fn downloadable(m: &wa::Message) -> Option<&dyn Downloadable> {
    let b = m.get_base_message();
    if let Some(x) = b.image_message.as_option() {
        return Some(x);
    }
    if let Some(x) = b.video_message.as_option().or(b.ptv_message.as_option()) {
        return Some(x);
    }
    if let Some(x) = b.audio_message.as_option() {
        return Some(x);
    }
    if let Some(x) = b.document_message.as_option() {
        return Some(x);
    }
    if let Some(x) = b.sticker_message.as_option() {
        return Some(x);
    }
    None
}

fn read_attachment(file: &Option<Attachment>) -> Result<Option<Vec<u8>>> {
    match file.as_ref().and_then(|f| f.path.as_ref()) {
        Some(p) => Ok(Some(std::fs::read(p)?)),
        None => Ok(None),
    }
}

impl Inner {
    /// Uploads (if needed), builds and sends one of our own messages under its local id.
    pub(crate) async fn send(&self, app: &Arc<App>, client: &Arc<Client>, m: &Message) -> Result<()> {
        let to = parse_jid(&m.chat)?;
        let timer = app.chat(&m.chat).map(|c| c.ephemeral).unwrap_or(0);
        let mut secret: Option<Vec<u8>> = None;
        let mut proto = match &m.content {
            Content::Poll { question, options, selectable } => {
                let poll = wa::message::PollCreationMessage {
                    name: Some(question.clone()),
                    options: options
                        .iter()
                        .map(|o| wa::message::poll_creation_message::Option { option_name: Some(o.name.clone()), ..Default::default() })
                        .collect(),
                    selectable_options_count: Some(*selectable),
                    ..Default::default()
                };
                secret = Some(random_secret());
                if *selectable == 1 {
                    wa::Message { poll_creation_message_v3: MessageField::some(poll), ..Default::default() }
                } else {
                    wa::Message { poll_creation_message: MessageField::some(poll), ..Default::default() }
                }
            }
            Content::Event { name, description, location, start, end, .. } => {
                secret = Some(random_secret());
                let ev = wa::message::EventMessage {
                    name: Some(name.clone()),
                    description: (!description.is_empty()).then(|| description.clone()),
                    start_time: Some(start / 1000),
                    end_time: (*end > 0).then_some(end / 1000),
                    location: if location.is_empty() {
                        MessageField::default()
                    } else {
                        MessageField::some(wa::message::LocationMessage { name: Some(location.clone()), ..Default::default() })
                    },
                    ..Default::default()
                };
                wa::Message { event_message: MessageField::some(ev), ..Default::default() }
            }
            content => {
                let mut uploaded = None;
                if let Some(media) = content.media() {
                    let token = media.token.as_deref().ok_or_else(|| anyhow!("the file is missing"))?;
                    let path = app.media.path(token).ok_or_else(|| anyhow!("the file is missing"))?;
                    let bytes = tokio::fs::read(&path).await?;
                    let up = err(client.upload(bytes, media_type(content), Default::default()).await)?;
                    uploaded = Some(Uploaded {
                        url: up.url,
                        direct_path: up.direct_path,
                        media_key: up.media_key.to_vec(),
                        file_enc_sha256: up.file_enc_sha256.to_vec(),
                        file_sha256: up.file_sha256.to_vec(),
                        file_length: up.file_length,
                        media_key_timestamp: up.media_key_timestamp,
                    });
                }
                let thumb = match content {
                    Content::Image { thumb, .. } | Content::Video { thumb, .. } | Content::Document { thumb, .. } => {
                        thumb.as_deref().map(convert::unb64)
                    }
                    _ => None,
                };
                let built = convert::build(m, uploaded.as_ref(), thumb, timer)?;
                if m.view_once {
                    wa::Message {
                        view_once_message: MessageField::some(wa::message::FutureProofMessage {
                            message: MessageField::some(built),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                } else {
                    built
                }
            }
        };
        if let Some(s) = &secret {
            proto.message_context_info =
                MessageField::some(wa::MessageContextInfo { message_secret: Some(s.clone()), ..Default::default() });
        }
        if convert::needs_raw(&m.content) {
            let mut stored = Stored::of(&proto, secret.as_deref());
            stored.sender = client.pn().map(|j| jid_str(&j)).unwrap_or_default();
            let _ = app.store.set_media_params(&m.chat, &m.id, &stored);
        }
        let mut options = SendOptions::default().with_message_id(m.id.clone());
        if timer > 0 {
            options = options.with_ephemeral_expiration(timer);
        }
        err(client.send_message_with_options(to, proto, options).await)?;
        Ok(())
    }

    fn stored(&self, app: &App, m: &Message) -> Result<Stored> {
        app.store.media_params::<Stored>(&m.chat, &m.id).ok_or_else(|| anyhow!("This message is no longer available on WhatsApp's servers"))
    }

    /// The address the creator of a poll/event was known by when it was sent.
    fn creator_of(&self, stored: &Stored, m: &Message) -> Result<WaJid> {
        if !stored.sender.is_empty() {
            if let Ok(j) = parse_jid(&bare(&stored.sender)) {
                return Ok(j);
            }
        }
        parse_jid(&self.wire_member(&m.chat, &m.sender))
    }

    pub(crate) async fn exec(self: &Arc<Self>, app: &Arc<App>, client: &Arc<Client>, op: NetOp) -> Result<Value> {
        match op {
            NetOp::Pair { .. } | NetOp::Logout | NetOp::DemoTrigger { .. } => bail!("handled elsewhere"),

            // ── messages ──
            NetOp::Send { msg } => {
                self.send(app, client, &msg).await?;
                Ok(Value::Null)
            }
            NetOp::React { target, emoji } => {
                err(client.send_reaction(parse_jid(&target.chat)?, self.key_of(&target), &emoji).await)?;
                Ok(Value::Null)
            }
            NetOp::Edit { target, text } => {
                let to = parse_jid(&target.chat)?;
                let content = match &target.content {
                    Content::Text { .. } => wa::Message { conversation: Some(text.clone()), ..Default::default() },
                    _ => {
                        // Captions are edited by resending the media message with the new caption.
                        let mut raw = self.stored(app, &target)?.message().ok_or_else(|| anyhow!("can't edit this message"))?;
                        if let Some(x) = raw.image_message.as_option_mut() {
                            x.caption = Some(text.clone());
                        } else if let Some(x) = raw.video_message.as_option_mut() {
                            x.caption = Some(text.clone());
                        } else if let Some(x) = raw.document_message.as_option_mut() {
                            x.caption = Some(text.clone());
                        }
                        raw
                    }
                };
                err(client.edit_message(to, target.id.clone(), content).await)?;
                Ok(Value::Null)
            }
            NetOp::Revoke { target } => {
                let to = parse_jid(&target.chat)?;
                let kind = match self.participant_of(&target) {
                    Some(original_sender) => RevokeType::Admin { original_sender },
                    None => RevokeType::Sender,
                };
                err(client.revoke_message(to, target.id.clone(), kind).await)?;
                Ok(Value::Null)
            }
            NetOp::DeleteForMe { targets } => {
                for m in targets {
                    let chat = parse_jid(&m.chat)?;
                    let who = self.participant_of(&m);
                    err(client.chat_actions().delete_message_for_me(&chat, who.as_ref(), &m.id, m.from_me, true, Some(m.ts / 1000)).await)?;
                }
                Ok(Value::Null)
            }
            NetOp::Pin { target, secs } => {
                let chat = parse_jid(&target.chat)?;
                let key = self.key_of(&target);
                if secs == 0 {
                    err(client.unpin_message(chat, key).await)?;
                } else {
                    let d = if secs <= 86_400 { PinDuration::Hours24 } else if secs <= 7 * 86_400 { PinDuration::Days7 } else { PinDuration::Days30 };
                    err(client.pin_message(chat, key, d).await)?;
                }
                Ok(Value::Null)
            }
            NetOp::Star { targets, on } => {
                for m in targets {
                    let chat = parse_jid(&m.chat)?;
                    let who = self.participant_of(&m);
                    let actions = client.chat_actions();
                    if on {
                        err(actions.star_message(&chat, who.as_ref(), &m.id, m.from_me).await)?;
                    } else {
                        err(actions.unstar_message(&chat, who.as_ref(), &m.id, m.from_me).await)?;
                    }
                }
                Ok(Value::Null)
            }
            NetOp::Vote { target, options } => {
                let stored = self.stored(app, &target)?;
                let creator = self.creator_of(&stored, &target)?;
                let secret = convert::unb64(&stored.secret);
                if secret.is_empty() {
                    bail!("This poll can only be voted on from your phone");
                }
                err(client.polls().vote(parse_jid(&target.chat)?, &target.id, &creator, &secret, &options).await)?;
                Ok(Value::Null)
            }
            NetOp::Rsvp { target, response } => {
                use wa::message::event_response_message::EventResponseType as R;
                let stored = self.stored(app, &target)?;
                let creator = self.creator_of(&stored, &target)?;
                let secret = convert::unb64(&stored.secret);
                if secret.is_empty() {
                    bail!("You can only respond to this event from your phone");
                }
                let r = match response.as_str() {
                    "going" => R::GOING,
                    "maybe" => R::MAYBE,
                    "declined" => R::NOT_GOING,
                    _ => R::UNKNOWN,
                };
                err(client.events().respond(parse_jid(&target.chat)?, &target.id, &creator, &secret, r, None).await)?;
                Ok(Value::Null)
            }
            NetOp::MarkRead { chat, msgs } => {
                let to = parse_jid(&chat)?;
                if is_group(&chat) {
                    // Group receipts are addressed per sender.
                    let mut by_sender: std::collections::HashMap<String, Vec<&str>> = Default::default();
                    for m in &msgs {
                        by_sender.entry(self.wire_member(&chat, &m.sender)).or_default().push(&m.id);
                    }
                    for (sender, ids) in by_sender {
                        let who = parse_jid(&sender)?;
                        client.mark_as_read(&to, Some(&who), &ids).await?;
                    }
                } else {
                    let ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
                    client.mark_as_read(&to, None, &ids).await?;
                }
                Ok(Value::Null)
            }
            NetOp::MarkPlayed { target } => {
                let who = self.participant_of(&target);
                client.mark_as_played(&parse_jid(&target.chat)?, who.as_ref(), &[target.id.as_str()]).await?;
                Ok(Value::Null)
            }
            NetOp::Download { target } => {
                let stored = self.stored(app, &target)?;
                let raw = stored.message().ok_or_else(|| anyhow!("can't read this message's media"))?;
                let ext = match &target.content {
                    Content::Document { name, media, .. } => crate::media::ext_for(&media.mime, name),
                    c => crate::media::ext_for(&c.media().map(|m| m.mime.clone()).unwrap_or_default(), ""),
                };
                let folder = if matches!(target.content, Content::Sticker { .. }) { "s" } else { "m" };
                let (token, path) = app.media.fresh(folder, &ext)?;
                let file = std::fs::File::create(&path)?;
                let d = downloadable(&raw).ok_or_else(|| anyhow!("nothing to download"))?;
                match client.download_to_writer(d, file).await {
                    Ok(_) => Ok(json!({ "token": token })),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(anyhow!("{e}"))
                    }
                }
            }
            NetOp::History { chat, oldest } => {
                if let Some(m) = oldest {
                    client.fetch_message_history(&parse_jid(&chat)?, &m.id, m.from_me, m.ts, 50).await?;
                }
                Ok(Value::Null)
            }

            // ── chats ──
            NetOp::Chat { chat, action, last } => {
                let jid = parse_jid(&chat)?;
                let range = last.as_ref().map(|m| {
                    whatsapp_rust::features::message_range(m.ts / 1000, None, vec![(self.key_of(m), m.ts / 1000)])
                });
                let actions = client.chat_actions();
                match action {
                    ChatAction::Archive(true) => err(actions.archive_chat(&jid, range).await)?,
                    ChatAction::Archive(false) => err(actions.unarchive_chat(&jid, range).await)?,
                    ChatAction::Pin(true) => err(actions.pin_chat(&jid).await)?,
                    ChatAction::Pin(false) => err(actions.unpin_chat(&jid).await)?,
                    ChatAction::Mute(0) => err(actions.unmute_chat(&jid).await)?,
                    ChatAction::Mute(-1) => err(actions.mute_chat(&jid).await)?,
                    ChatAction::Mute(until) => err(actions.mute_chat_until(&jid, until).await)?,
                    ChatAction::MarkRead(read) => err(actions.mark_chat_as_read(&jid, read, range).await)?,
                    ChatAction::Delete => err(actions.delete_chat(&jid, true, range).await)?,
                    ChatAction::Clear => err(actions.clear_chat(&jid, true, true, range).await)?,
                }
                Ok(Value::Null)
            }
            NetOp::Typing { chat, state } => {
                let jid = parse_jid(&chat)?;
                let cs = client.chatstate();
                match state.as_str() {
                    "typing" => err(cs.send_composing(&jid).await)?,
                    "recording" => err(cs.send_recording(&jid).await)?,
                    _ => err(cs.send_paused(&jid).await)?,
                }
                Ok(Value::Null)
            }
            NetOp::Subscribe { jid } => {
                err(client.presence().subscribe(parse_jid(&jid)?).await)?;
                Ok(Value::Null)
            }
            NetOp::Available { on } => {
                if on {
                    err(client.presence().set_available().await)?;
                } else {
                    err(client.presence().set_unavailable().await)?;
                }
                Ok(Value::Null)
            }
            NetOp::Ephemeral { chat, secs } => {
                let jid = parse_jid(&chat)?;
                if is_group(&chat) {
                    err(client.groups().set_ephemeral(jid, secs).await)?;
                } else {
                    err(client.set_chat_disappearing_timer(jid, secs).await)?;
                }
                Ok(Value::Null)
            }

            // ── people ──
            NetOp::Avatar { jid } => {
                self.fetch_avatar(app, client, &jid, false).await;
                Ok(Value::Null)
            }
            NetOp::CheckNumber { phone } => {
                let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
                let res = err(client.contacts().is_on_whatsapp(&[WaJid::pn(digits.as_str())]).await)?;
                let hit = res.into_iter().find(|r| r.is_registered);
                Ok(match hit {
                    Some(r) => {
                        let jid = self.canon(&r.jid, r.pn_jid.as_ref().or(r.lid.as_ref()));
                        json!({ "exists": true, "jid": jid })
                    }
                    None => json!({ "exists": false, "jid": format!("{digits}@{USER_SERVER}") }),
                })
            }
            NetOp::Block { jid, on } => {
                let j = parse_jid(&jid)?;
                if on {
                    err(client.blocking().block(&j).await)?;
                } else {
                    err(client.blocking().unblock(&j).await)?;
                }
                Ok(Value::Null)
            }
            NetOp::SaveContact { jid, name } => {
                let first = name.split_whitespace().next().map(String::from);
                err(client.chat_actions().save_contact(&parse_jid(&jid)?, Some(name), first, true).await)?;
                Ok(Value::Null)
            }
            NetOp::Profile(op) => {
                match op {
                    ProfileOp::Name { text } => err(client.profile().set_push_name(text.trim()).await)?,
                    ProfileOp::About { text } => err(client.profile().set_status_text(&text).await)?,
                    ProfileOp::Photo { file } => match read_attachment(&file)? {
                        Some(bytes) => drop(err(client.profile().set_profile_picture(bytes).await)?),
                        None => drop(err(client.profile().remove_profile_picture().await)?),
                    },
                }
                Ok(Value::Null)
            }
            NetOp::PrivacyGet => Ok(Value::Null),
            NetOp::PrivacySet { privacy } => {
                use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory as C, PrivacyValue as V};
                let old = app.store.meta::<Privacy>("privacy").unwrap_or_default();
                let val = |s: &str| match s {
                    "contacts" => V::Contacts,
                    "none" => V::None,
                    "match_last_seen" => V::MatchLastSeen,
                    _ => V::All,
                };
                let changes = [
                    (old.last_seen != privacy.last_seen, C::Last, val(&privacy.last_seen)),
                    (old.online != privacy.online, C::Online, val(&privacy.online)),
                    (old.profile_photo != privacy.profile_photo, C::Profile, val(&privacy.profile_photo)),
                    (old.about != privacy.about, C::Status, val(&privacy.about)),
                    (old.group_add != privacy.group_add, C::GroupAdd, val(&privacy.group_add)),
                    (old.read_receipts != privacy.read_receipts, C::ReadReceipts, if privacy.read_receipts { V::All } else { V::None }),
                ];
                for (changed, category, value) in changes {
                    if changed {
                        err(client.set_privacy_setting(category, value).await)?;
                    }
                }
                if old.default_ephemeral != privacy.default_ephemeral {
                    err(client.set_default_disappearing_mode(privacy.default_ephemeral).await)?;
                }
                Ok(Value::Null)
            }
            NetOp::RejectCall { id, .. } => {
                let call = lock(&self.calls).remove(&id);
                if let Some(call) = call {
                    err(client.voip().reject_call(&id, &call.from, &call.creator).await)?;
                }
                Ok(Value::Null)
            }

            NetOp::Group(op) => self.group_op(app, client, op).await,
            NetOp::Community(op) => self.community_op(app, client, op).await,
        }
    }

    async fn group_op(&self, app: &Arc<App>, client: &Arc<Client>, op: GroupOp) -> Result<Value> {
        use whatsapp_rust::features::{
            GroupCreateOptions, GroupDescription, GroupParticipantOptions, GroupSubject, MemberAddMode, MembershipApprovalMode,
        };
        let me = app.me();
        let groups = client.groups();
        let members = |jids: &[String]| -> Result<Vec<WaJid>> { jids.iter().map(|j| parse_jid(j)).collect() };
        Ok(match op {
            GroupOp::Create { subject, participants, parent } => {
                let mut options = GroupCreateOptions::new(subject.trim());
                options.participants = members(&participants)?
                    .into_iter()
                    .map(|jid| GroupParticipantOptions { jid, phone_number: None, privacy: None })
                    .collect();
                options.linked_parent = parent.as_deref().map(parse_jid).transpose()?;
                let res = err(groups.create_group(options).await)?;
                let g = group_from(self, &me, &res.metadata);
                let id = g.id.clone();
                merge_group(app, g);
                json!({ "id": id })
            }
            GroupOp::Refresh { id } => {
                let md = err(groups.get_metadata(&parse_jid(&id)?).await)?;
                merge_group(app, group_from(self, &me, &md));
                Value::Null
            }
            GroupOp::Subject { id, text } => {
                err(groups.set_subject(parse_jid(&id)?, err(GroupSubject::new(text.trim()))?).await)?;
                Value::Null
            }
            GroupOp::Description { id, text } => {
                let d = if text.trim().is_empty() { None } else { Some(err(GroupDescription::new(text))?) };
                err(groups.set_description(parse_jid(&id)?, d, whatsapp_rust::features::PreviousDescription::Resolve).await)?;
                Value::Null
            }
            GroupOp::Setting { id, key, on } => {
                let jid = parse_jid(&id)?;
                match key.as_str() {
                    "announce" => err(groups.set_announce(jid, on).await)?,
                    "locked" => err(groups.set_locked(jid, on).await)?,
                    "approval" => err(groups.set_membership_approval(jid, if on { MembershipApprovalMode::On } else { MembershipApprovalMode::Off }).await)?,
                    "membersCanAdd" => err(groups.set_member_add_mode(jid, if on { MemberAddMode::AllMemberAdd } else { MemberAddMode::AdminAdd }).await)?,
                    other => bail!("unknown group setting {other}"),
                }
                Value::Null
            }
            GroupOp::Participants { id, action, jids } => {
                let jid = parse_jid(&id)?;
                let wire: Vec<String> = jids.iter().map(|j| if action == "add" { j.clone() } else { self.wire_member(&id, j) }).collect();
                let list = members(&wire)?;
                match action.as_str() {
                    "add" => drop(err(groups.add_participants(jid, &list).await)?),
                    "remove" => drop(err(groups.remove_participants(jid, &list).await)?),
                    "promote" => drop(err(groups.promote_participants(jid, &list).await)?),
                    "demote" => drop(err(groups.demote_participants(jid, &list).await)?),
                    other => bail!("unknown participant action {other}"),
                }
                Value::Null
            }
            GroupOp::Invite { id, reset } => {
                let link = err(groups.get_invite_link(parse_jid(&id)?, reset).await)?;
                let code = link.rsplit('/').next().unwrap_or(&link).to_string();
                json!({ "code": code })
            }
            GroupOp::InviteInfo { code } => {
                let md = err(groups.get_invite_info(code.trim()).await)?;
                json!({
                    "id": jid_str(&md.id),
                    "subject": md.subject,
                    "size": md.size.unwrap_or(md.participants.len() as u32),
                    "description": md.description,
                })
            }
            GroupOp::Join { code } => {
                use whatsapp_rust::features::JoinGroupResult as J;
                match err(groups.join_with_invite_code(code.trim()).await)? {
                    J::Joined(jid) => {
                        if let Ok(md) = groups.get_metadata(&jid).await {
                            merge_group(app, group_from(self, &me, &md));
                        }
                        json!({ "id": jid_str(&jid) })
                    }
                    J::PendingApproval(jid) => json!({ "id": Value::Null, "pending": jid_str(&jid) }),
                }
            }
            GroupOp::Leave { id } => {
                err(groups.leave(parse_jid(&id)?).await)?;
                Value::Null
            }
            GroupOp::RequestAction { id, jids, approve } => {
                let jid = parse_jid(&id)?;
                let list = members(&jids)?;
                if approve {
                    drop(err(groups.approve_membership_requests(jid, &list).await)?);
                } else {
                    drop(err(groups.reject_membership_requests(jid, &list).await)?);
                }
                Value::Null
            }
            GroupOp::Icon { id, file } => {
                let jid = parse_jid(&id)?;
                match read_attachment(&file)? {
                    Some(bytes) => drop(err(groups.set_profile_picture(jid, bytes).await)?),
                    None => drop(err(groups.remove_profile_picture(jid).await)?),
                }
                Value::Null
            }
        })
    }

    async fn community_op(&self, app: &Arc<App>, client: &Arc<Client>, op: CommunityOp) -> Result<Value> {
        let me = app.me();
        let community = client.community();
        let list = |jids: &[String]| -> Result<Vec<WaJid>> { jids.iter().map(|j| parse_jid(j)).collect() };
        Ok(match op {
            CommunityOp::Create { name, description } => {
                let mut options = whatsapp_rust::features::CreateCommunityOptions::new(name.trim());
                options.description = (!description.trim().is_empty()).then_some(description);
                let res = err(community.create(options).await)?;
                let g = group_from(self, &me, &res.metadata);
                let id = g.id.clone();
                merge_group(app, g);
                // The announcement group is created with the community; pick it up.
                let mut announcement = Value::Null;
                if let Ok(subs) = community.get_subgroups(&res.metadata.id).await {
                    for s in subs.iter().filter(|s| s.is_default_sub_group) {
                        if let Ok(md) = client.groups().get_metadata(&s.id).await {
                            let mut sg = group_from(self, &me, &md);
                            sg.parent = Some(id.clone());
                            sg.is_announcement = true;
                            announcement = json!(sg.id);
                            merge_group(app, sg);
                        }
                    }
                }
                json!({ "id": id, "announcement": announcement })
            }
            CommunityOp::Refresh { .. } => {
                self.refresh_groups(app, client).await;
                Value::Null
            }
            CommunityOp::Link { id, groups } => {
                err(community.link_subgroups(parse_jid(&id)?, &list(&groups)?).await)?;
                Value::Null
            }
            CommunityOp::Unlink { id, groups } => {
                err(community.unlink_subgroups(parse_jid(&id)?, &list(&groups)?, false).await)?;
                Value::Null
            }
            CommunityOp::JoinSubgroup { id, group } => {
                let md = err(community.join_subgroup(parse_jid(&id)?, parse_jid(&group)?).await)?;
                let mut g = group_from(self, &me, &md);
                g.parent = Some(id);
                g.joined = true;
                merge_group(app, g);
                Value::Null
            }
            CommunityOp::Deactivate { id } => {
                err(community.deactivate(parse_jid(&id)?).await)?;
                Value::Null
            }
        })
    }
}
