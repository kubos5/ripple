//! Exercises the live backend's ingest paths without a WhatsApp connection.

use std::sync::Arc;

use whatsapp_rust::wacore::event::encrypt_event_response_with_secret;
use whatsapp_rust::wacore::poll::{compute_option_hash, encrypt_poll_vote_with_secret};
use whatsapp_rust::waproto::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

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
    wa::Message { conversation: Some(s.into()), ..Default::default() }
}

fn key(id: &str) -> MessageField<wa::MessageKey> {
    MessageField::some(wa::MessageKey { id: Some(id.into()), ..Default::default() })
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

    let reaction = wa::Message {
        reaction_message: MessageField::some(wa::message::ReactionMessage { key: key("M1"), text: Some("👍".into()), ..Default::default() }),
        ..Default::default()
    };
    net.inner.apply_message(&app, &reaction, incoming("R1", ANA, ANA, ANA));
    assert_eq!(app.store.message(ANA, "M1").unwrap().reactions[0].emoji, "👍");
    assert!(app.store.message(ANA, "R1").is_none(), "reactions are not messages");

    let edit = wa::Message {
        protocol_message: MessageField::some(wa::message::ProtocolMessage {
            key: key("M1"),
            r#type: Some(wa::message::protocol_message::Type::MESSAGE_EDIT),
            edited_message: MessageField::some(text("hello, edited")),
            ..Default::default()
        }),
        ..Default::default()
    };
    net.inner.apply_message(&app, &edit, incoming("E1", ANA, ANA, ANA));
    let m = app.store.message(ANA, "M1").unwrap();
    assert_eq!(m.content.text(), "hello, edited");
    assert!(m.edited > 0);

    let revoke = wa::Message {
        protocol_message: MessageField::some(wa::message::ProtocolMessage {
            key: key("M1"),
            r#type: Some(wa::message::protocol_message::Type::REVOKE),
            ..Default::default()
        }),
        ..Default::default()
    };
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
    let msg = wa::Message {
        image_message: MessageField::some(wa::message::ImageMessage {
            mimetype: Some("image/jpeg".into()),
            direct_path: Some("/v/t62.7118-24/abc".into()),
            media_key: Some(vec![7; 32]),
            file_length: Some(4096),
            width: Some(800),
            height: Some(600),
            caption: Some("sunset".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
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
    let poll = wa::Message {
        poll_creation_message_v3: MessageField::some(wa::message::PollCreationMessage {
            name: Some("Tea?".into()),
            options: ["Yes", "No"]
                .iter()
                .map(|o| wa::message::poll_creation_message::Option { option_name: Some(o.to_string()), ..Default::default() })
                .collect(),
            selectable_options_count: Some(1),
            ..Default::default()
        }),
        message_context_info: MessageField::some(wa::MessageContextInfo { message_secret: Some(secret.clone()), ..Default::default() }),
        ..Default::default()
    };
    net.inner.apply_message(&app, &poll, incoming("POLL1", GROUP, ME, ME));

    // Ana votes "No" from a device that knows her only by her hidden id.
    let (payload, iv) =
        encrypt_poll_vote_with_secret(&[compute_option_hash("No").to_vec()], &secret, "POLL1", ME, ANA_LID).unwrap();
    let vote = wa::Message {
        poll_update_message: MessageField::some(wa::message::PollUpdateMessage {
            poll_creation_message_key: key("POLL1"),
            vote: MessageField::some(wa::message::PollEncValue { enc_payload: Some(payload), enc_iv: Some(iv.to_vec()), ..Default::default() }),
            ..Default::default()
        }),
        ..Default::default()
    };
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
    let event = wa::Message {
        event_message: MessageField::some(wa::message::EventMessage { name: Some("Picnic".into()), start_time: Some(1_800_000_000), ..Default::default() }),
        message_context_info: MessageField::some(wa::MessageContextInfo { message_secret: Some(secret.clone()), ..Default::default() }),
        ..Default::default()
    };
    net.inner.apply_message(&app, &event, incoming("EV1", GROUP, ME, ME));
    let response = wa::message::EventResponseMessage { response: Some(R::GOING), ..Default::default() };
    let (payload, iv) = encrypt_event_response_with_secret(&response, &secret, "EV1", ME, ANA).unwrap();
    let reply = wa::Message {
        enc_event_response_message: MessageField::some(wa::message::EncEventResponseMessage {
            event_creation_message_key: key("EV1"),
            enc_payload: Some(payload),
            enc_iv: Some(iv.to_vec()),
            ..Default::default()
        }),
        ..Default::default()
    };
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
    let web = |id: &str, from_me: bool, secs: u64, body: &str| wa::HistorySyncMsg {
        message: MessageField::some(wa::WebMessageInfo {
            key: MessageField::some(wa::MessageKey { id: Some(id.into()), from_me: Some(from_me), remote_jid: Some(ANA_LID.into()), ..Default::default() }),
            message: MessageField::some(text(body)),
            message_timestamp: Some(secs),
            status: from_me.then_some(wa::web_message_info::Status::READ),
            ..Default::default()
        }),
        ..Default::default()
    };
    let hs = wa::HistorySync {
        conversations: vec![wa::Conversation {
            id: ANA_LID.into(),
            pn_jid: Some(ANA.into()),
            messages: vec![web("H2", true, 1_700_000_100, "on my way"), web("H1", false, 1_700_000_000, "where are you?")],
            unread_count: Some(0),
            archived: Some(true),
            pinned: Some(1_700_000_500),
            mute_end_time: Some(0),
            ephemeral_expiration: Some(604_800),
            ..Default::default()
        }],
        pushnames: vec![wa::Pushname { id: Some(ANA.into()), pushname: Some("Ana".into()), ..Default::default() }],
        progress: Some(40),
        ..Default::default()
    };
    assert_eq!(net.inner.apply_history(&app, &hs), Some(40));

    // The conversation arrived under a hidden id but is filed under the phone number.
    assert!(app.chat(ANA_LID).is_none());
    let chat = app.chat(ANA).unwrap();
    assert!(chat.archived && chat.pinned > 0 && chat.ephemeral == 604_800);
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
    let member = |jid: Jid, pn: Option<Jid>, kind: ParticipantType| GroupParticipant {
        jid,
        phone_number: pn,
        lid: None,
        username: None,
        participant_type: kind,
        details: None,
    };
    let mut md = GroupMetadata::default();
    md.id = GROUP.parse().unwrap();
    md.subject = "Climbers".into();
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
