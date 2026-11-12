//! Exercises the live backend's ingest paths without a WhatsApp connection.

use std::sync::Arc;

use whatsapp_rust::wacore::event::encrypt_event_response_with_secret;
use whatsapp_rust::wacore::poll::{compute_option_hash, encrypt_poll_vote_with_secret};
use whatsapp_rust::waproto::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

use super::convert::{self, pb};
use super::events::Incoming;
use super::{LiveNet, group_from, merge_group};
use crate::app::App;
use crate::model::*;

const ME: &str = "15550100000@s.whatsapp.net";
const ANA: &str = "15550100001@s.whatsapp.net";
const ANA_LID: &str = "9990001@lid";
const GROUP: &str = "120363000000000099@g.us";

fn setup() -> (Arc<App>, LiveNet, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ripple-wa-test-{}", crate::media::random_hex(5)));
    let (tx, _) = tokio::sync::broadcast::channel(256);
    let app = App::new(dir.clone(), tx, false).unwrap();
    app.set_me(ME, Some("Sam".into()));
    (app, LiveNet::new(), dir)
}

fn incoming(id: &str, chat: &str, sender: &str, raw: &str) -> Incoming {
    Incoming {
        id: id.into(),
        chat: chat.into(),
        sender: sender.into(),
        sender_raw: raw.into(),
        sender_alt: None,
        from_me: sender == ME,
        ts: now_ms(),
        push_name: String::new(),
        timer: None,
        live: true,
    }
}

fn text(s: &str) -> wa::Message {
    pb!(wa::Message { conversation: Some(s.into()) })
}

fn key(id: &str) -> MessageField<wa::MessageKey> {
    MessageField::some(pb!(wa::MessageKey { id: Some(id.into()) }))
}

#[tokio::test]
async fn text_then_reaction_edit_and_revoke() {
    let (app, net, dir) = setup();
    let mut first = incoming("M1", ANA, ANA, ANA);
    first.push_name = "Ana".into();
    net.inner.apply_message(&app, &text("hello there"), first);

    let m = app.store.message(ANA, "M1").unwrap();
    assert_eq!(m.content.text(), "hello there");
    assert_eq!(app.chat(ANA).unwrap().unread, 1);
    assert_eq!(app.chat(ANA).unwrap().name, "Ana", "push name names the chat");

    let reaction = pb!(wa::Message {
        reaction_message: MessageField::some(pb!(wa::message::ReactionMessage { key: key("M1"), text: Some("👍".into()) })) });
    net.inner.apply_message(&app, &reaction, incoming("R1", ANA, ANA, ANA));
    assert_eq!(app.store.message(ANA, "M1").unwrap().reactions[0].emoji, "👍");
    assert!(app.store.message(ANA, "R1").is_none(), "reactions are not messages");

    let edit = pb!(wa::Message {
        protocol_message: MessageField::some(pb!(wa::message::ProtocolMessage {
            key: key("M1"),
            r#type: Some(wa::message::protocol_message::Type::MESSAGE_EDIT),
            edited_message: MessageField::some(text("hello, edited")) })) });
    net.inner.apply_message(&app, &edit, incoming("E1", ANA, ANA, ANA));
    let m = app.store.message(ANA, "M1").unwrap();
    assert_eq!(m.content.text(), "hello, edited");
    assert!(m.edited > 0);

    let revoke = pb!(wa::Message {
        protocol_message: MessageField::some(pb!(wa::message::ProtocolMessage {
            key: key("M1"),
            r#type: Some(wa::message::protocol_message::Type::REVOKE) })) });
    net.inner.apply_message(&app, &revoke, incoming("V1", ANA, ANA, ANA));
    assert_eq!(app.store.message(ANA, "M1").unwrap().content.kind(), "deleted");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn offline_messages_count_as_unread_and_duplicates_are_ignored() {
    let (app, net, dir) = setup();
    let mut i = incoming("M1", ANA, ANA, ANA);
    i.live = false;
    net.inner.apply_message(&app, &text("while you were away"), i);
    net.inner.apply_message(&app, &text("while you were away"), incoming("M1", ANA, ANA, ANA));
    assert_eq!(app.chat(ANA).unwrap().unread, 1);
    assert_eq!(app.store.messages(ANA, None, 10).len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn media_keeps_its_raw_form_and_disappearing_timer() {
    let (app, net, dir) = setup();
    let msg = pb!(wa::Message {
        image_message: MessageField::some(pb!(wa::message::ImageMessage {
            mimetype: Some("image/jpeg".into()),
            direct_path: Some("/v/t62.7118-24/abc".into()),
            media_key: Some(vec![7; 32]),
            file_length: Some(4096),
            width: Some(800),
            height: Some(600),
            caption: Some("sunset".into()) })) });
    let mut i = incoming("IMG1", ANA, ANA, ANA_LID);
    i.timer = Some(86_400);
    let ts = i.ts;
    net.inner.apply_message(&app, &msg, i);
    let m = app.store.message(ANA, "IMG1").unwrap();
    assert_eq!(m.content.media().unwrap().state, MediaState::Remote);
    assert_eq!(m.expires, ts + 86_400_000);
    assert_eq!(app.chat(ANA).unwrap().ephemeral, 86_400);
    let stored: super::convert::Stored = app.store.media_params(ANA, "IMG1").unwrap();
    assert_eq!(stored.sender, ANA_LID);
    let raw = stored.message().unwrap();
    assert_eq!(raw.image_message.as_option().unwrap().media_key.as_deref(), Some(&[7u8; 32][..]));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn poll_votes_decrypt_even_when_the_voter_is_addressed_by_lid() {
    let (app, net, dir) = setup();
    net.inner.learn(ANA_LID, ANA);
    let secret = vec![42u8; 32];
    let poll = pb!(wa::Message {
        poll_creation_message_v3: MessageField::some(pb!(wa::message::PollCreationMessage {
            name: Some("Tea?".into()),
            options: ["Yes", "No"]
                .iter()
                .map(|o| pb!(wa::message::poll_creation_message::Option { option_name: Some(o.to_string()) }))
                .collect(),
            selectable_options_count: Some(1) })),
        message_context_info: MessageField::some(pb!(wa::MessageContextInfo { message_secret: Some(secret.clone()) })) });
    net.inner.apply_message(&app, &poll, incoming("POLL1", GROUP, ME, ME));

    // Ana votes "No" from a device that knows her only by her hidden id.
    let (payload, iv) =
        encrypt_poll_vote_with_secret(&[compute_option_hash("No").to_vec()], &secret, "POLL1", ME, ANA_LID).unwrap();
    let vote = pb!(wa::Message {
        poll_update_message: MessageField::some(pb!(wa::message::PollUpdateMessage {
            poll_creation_message_key: key("POLL1"),
            vote: MessageField::some(pb!(wa::message::PollEncValue { enc_payload: Some(payload), enc_iv: Some(iv.to_vec()) })) })) });
    net.inner.apply_message(&app, &vote, incoming("VOTE1", GROUP, ANA, ANA_LID));
    match app.store.message(GROUP, "POLL1").unwrap().content {
        Content::Poll { options, .. } => {
            assert!(options[0].voters.is_empty());
            assert_eq!(options[1].voters, vec![ANA.to_string()], "the vote is filed under her phone-number id");
        }
        other => panic!("expected a poll, got {}", other.kind()),
    }
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn event_responses_decrypt() {
    use wa::message::event_response_message::EventResponseType as R;
    let (app, net, dir) = setup();
    let secret = vec![9u8; 32];
    let event = pb!(wa::Message {
        event_message: MessageField::some(pb!(wa::message::EventMessage { name: Some("Picnic".into()), start_time: Some(1_800_000_000) })),
        message_context_info: MessageField::some(pb!(wa::MessageContextInfo { message_secret: Some(secret.clone()) })) });
    net.inner.apply_message(&app, &event, incoming("EV1", GROUP, ME, ME));
    let response = pb!(wa::message::EventResponseMessage { response: Some(R::GOING) });
    let (payload, iv) = encrypt_event_response_with_secret(&response, &secret, "EV1", ME, ANA).unwrap();
    let reply = pb!(wa::Message {
        enc_event_response_message: MessageField::some(pb!(wa::message::EncEventResponseMessage {
            event_creation_message_key: key("EV1"),
            enc_payload: Some(payload),
            enc_iv: Some(iv.to_vec()) })) });
    net.inner.apply_message(&app, &reply, incoming("RSVP1", GROUP, ANA, ANA));
    match app.store.message(GROUP, "EV1").unwrap().content {
        Content::Event { responses, start, .. } => {
            assert_eq!(responses.going, vec![ANA.to_string()]);
            assert_eq!(start, 1_800_000_000_000);
        }
        other => panic!("expected an event, got {}", other.kind()),
    }
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn history_chunk_fills_chats_names_and_state() {
    let (app, net, dir) = setup();
    let web = |id: &str, from_me: bool, secs: u64, body: &str| pb!(wa::HistorySyncMsg {
        message: MessageField::some(pb!(wa::WebMessageInfo {
            key: MessageField::some(pb!(wa::MessageKey { id: Some(id.into()), from_me: Some(from_me), remote_jid: Some(ANA_LID.into()) })),
            message: MessageField::some(text(body)),
            message_timestamp: Some(secs),
            status: from_me.then_some(wa::web_message_info::Status::READ) })) });
    let hs = pb!(wa::HistorySync {
        conversations: vec![pb!(wa::Conversation {
            id: ANA_LID.into(),
            pn_jid: Some(ANA.into()),
            messages: vec![web("H2", true, 1_700_000_100, "on my way"), web("H1", false, 1_700_000_000, "where are you?")],
            unread_count: Some(0),
            archived: Some(true),
            pinned: Some(1_700_000_500),
            mute_end_time: Some(4_102_444_800), // 2100-01-01, in seconds
            ephemeral_expiration: Some(604_800) })],
        pushnames: vec![pb!(wa::Pushname { id: Some(ANA.into()), pushname: Some("Ana".into()) })],
        progress: Some(40) });
    assert_eq!(net.inner.apply_history(&app, &hs), Some(40));

    // The conversation arrived under a hidden id but is filed under the phone number.
    assert!(app.chat(ANA_LID).is_none());
    let chat = app.chat(ANA).unwrap();
    assert!(chat.archived && chat.pinned > 0 && chat.ephemeral == 604_800);
    // History sync gives the mute end in seconds; it is kept in milliseconds.
    assert_eq!(chat.muted_until, 4_102_444_800_000);
    assert_eq!(convert::mute_until(-1), -1);
    assert_eq!(convert::mute_until(u64::MAX as i64), -1);
    assert_eq!(convert::mute_until(0), 0);
    assert_eq!(chat.last.as_ref().unwrap().id, "H2");
    assert_eq!(chat.name, "Ana");
    let msgs = app.store.messages(ANA, None, 10);
    assert_eq!(msgs.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["H1", "H2"]);
    assert_eq!(msgs[1].status, Status::Read);
    assert_eq!(msgs[0].sender, ANA);
    assert_eq!(net.inner.canon_s("9990001:4@lid"), ANA);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn group_metadata_maps_roles_and_lid_members() {
    use whatsapp_rust::Jid;
    use whatsapp_rust::features::{GroupMetadata, GroupParticipant, ParticipantType};
    let (app, net, dir) = setup();
    let member = |jid: Jid, pn: Option<Jid>, kind: ParticipantType| {
        let mut p = GroupParticipant::new(jid);
        p.phone_number = pn;
        p.participant_type = kind;
        p
    };
    let mut md = GroupMetadata::new(GROUP.parse().unwrap());
    md.subject = Some("Climbers".into());
    md.is_announcement = true;
    md.participants = vec![
        member(ME.parse().unwrap(), None, ParticipantType::Member),
        member(ANA_LID.parse().unwrap(), Some(ANA.parse().unwrap()), ParticipantType::SuperAdmin),
    ];
    let g = group_from(&net.inner, ME, &md);
    assert_eq!(g.role_of(ANA), Some(Role::Owner), "LID member is filed under her phone number");
    assert!(g.joined && g.announce);
    assert_eq!(net.inner.wire_member(GROUP, ANA), ANA_LID, "and addressed by LID inside this group");
    merge_group(&app, g);
    let chat = app.chat(GROUP).unwrap();
    assert_eq!(chat.name, "Climbers");
    assert!(chat.read_only, "members can't post in an announce-only group");
    std::fs::remove_dir_all(dir).ok();
}

fn sticker_message(hash: u8) -> wa::Message {
    pb!(wa::Message {
        sticker_message: MessageField::some(pb!(wa::message::StickerMessage {
            file_sha256: Some(vec![hash; 32]),
            file_enc_sha256: Some(vec![2; 32]),
            media_key: Some(vec![3; 32]),
            direct_path: Some("/v/t62.15575-24/s".into()),
            file_length: Some(4000),
            mimetype: Some("image/webp".into())
        }))
    })
}

#[tokio::test]
async fn sent_stickers_become_recents_but_received_ones_do_not() {
    let (app, net, dir) = setup();
    let mut mine = incoming("S1", ANA, ME, ME);
    mine.ts = 1_000;
    net.inner.apply_message(&app, &sticker_message(7), mine);
    net.inner.apply_message(&app, &sticker_message(8), incoming("S2", ANA, ANA, ANA));
    let lib = app.store.stickers();
    assert_eq!(lib.len(), 1, "only stickers the user sent count as recent");
    let s = &lib[0];
    assert_eq!(s.id, super::convert::b64(&[7; 32]));
    assert_eq!(s.used, 1_000);
    assert!(s.token.is_none(), "the file is fetched later, when the picker needs it");
    assert_eq!(s.remote.as_ref().unwrap().direct_path, "/v/t62.15575-24/s");
    // Sending it again moves it up; a favourite flag set meanwhile survives.
    app.store.update_sticker(&s.id, |x| x.favorite = true).unwrap();
    let mut again = incoming("S3", ANA, ME, ME);
    again.ts = 2_000;
    net.inner.apply_message(&app, &sticker_message(7), again);
    let s = app.store.sticker(&super::convert::b64(&[7; 32])).unwrap();
    assert_eq!((s.used, s.favorite), (2_000, true));
    // Removing it from recents on another device keeps the favourite.
    app.sticker_unrecent(&s.id, Some(2_000));
    let s = app.store.sticker(&s.id).unwrap();
    assert_eq!((s.used, s.favorite), (0, true));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn history_brings_the_phones_recent_stickers() {
    let (app, net, dir) = setup();
    let meta = pb!(wa::StickerMetadata {
        file_sha256: Some(vec![9; 32]),
        file_enc_sha256: Some(vec![2; 32]),
        media_key: Some(vec![3; 32]),
        direct_path: Some("/v/t62.15575-24/r".into()),
        file_length: Some(3000),
        width: Some(512),
        height: Some(512),
        last_sticker_sent_ts: Some(1_700_000_000)
    });
    let mut lottie = meta.clone();
    lottie.file_sha256 = Some(vec![10; 32]);
    lottie.is_lottie = Some(true);
    let hs = pb!(wa::HistorySync { recent_stickers: vec![meta, lottie] });
    net.inner.apply_history(&app, &hs);
    let lib = app.store.stickers();
    assert_eq!(lib.len(), 1, "lottie stickers can't be drawn and are skipped");
    assert_eq!(lib[0].used, 1_700_000_000_000, "seconds are normalised to milliseconds");
    std::fs::remove_dir_all(dir).ok();
}
