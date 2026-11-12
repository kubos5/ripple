//! The demo's starting state: people, groups, communities and conversations
//! that between them exercise every message type the UI can draw.

use std::sync::Arc;

use super::art;
use crate::app::App;
use crate::media;
use crate::model::*;

pub const ME: &str = "15550100000@s.whatsapp.net";
pub const MAYA: &str = "15550100101@s.whatsapp.net";
pub const LEO: &str = "15550100102@s.whatsapp.net";
pub const DAD: &str = "15550100103@s.whatsapp.net";
pub const PRIYA: &str = "15550100104@s.whatsapp.net";
pub const HANNAH: &str = "15550100105@s.whatsapp.net";
pub const TOMAS: &str = "15550100106@s.whatsapp.net";
pub const AIKO: &str = "15550100107@s.whatsapp.net";
pub const NOOR: &str = "15550100108@s.whatsapp.net";
pub const JONAS: &str = "15550100109@s.whatsapp.net";
pub const CHLOE: &str = "15550100110@s.whatsapp.net";
pub const BEN: &str = "15550100111@s.whatsapp.net";
pub const ZARA: &str = "15550100112@s.whatsapp.net";
pub const OLU: &str = "15550100113@s.whatsapp.net";
pub const INES: &str = "15550100114@s.whatsapp.net";
pub const STRANGER: &str = "447700900123@s.whatsapp.net";

pub const HIKERS: &str = "120363000000000001@g.us";
pub const CRIT: &str = "120363000000000002@g.us";
pub const FLAT: &str = "120363000000000003@g.us";
pub const BOOKS: &str = "120363000000000004@g.us";
pub const MAPLE: &str = "120363000000000010@g.us";
pub const MAPLE_NEWS: &str = "120363000000000011@g.us";
pub const MAPLE_SELL: &str = "120363000000000012@g.us";
pub const MAPLE_GARDEN: &str = "120363000000000013@g.us";
pub const MAPLE_SWAP: &str = "120363000000000014@g.us";
pub const MAPLE_EVENTS: &str = "120363000000000015@g.us";
pub const RUNNERS: &str = "120363000000000020@g.us";
pub const RUNNERS_NEWS: &str = "120363000000000021@g.us";
pub const RUNNERS_TEMPO: &str = "120363000000000022@g.us";

struct Seeder {
    app: Arc<App>,
    now: i64,
    n: u32,
    batch: Vec<Message>,
}

fn text(s: &str) -> Content {
    Content::Text { text: s.to_string(), preview: None }
}

fn system(event: &str, actor: Option<&str>, targets: &[&str], txt: &str) -> Content {
    Content::System {
        event: event.to_string(),
        actor: actor.map(String::from),
        targets: targets.iter().map(|s| s.to_string()).collect(),
        text: txt.to_string(),
    }
}

impl Seeder {
    /// Adds a message `mins` minutes before now and returns it for tweaking.
    fn add(&mut self, chat: &str, from: &str, mins: i64, content: Content) -> &mut Message {
        self.n += 1;
        let from_me = from == ME;
        self.batch.push(Message {
            id: format!("DEMO{:05}", self.n),
            chat: chat.to_string(),
            seq: 0,
            sender: from.to_string(),
            from_me,
            // Later messages in the same minute stay in order.
            ts: self.now - mins * 60_000 + self.n as i64,
            content,
            status: if from_me { Status::Read } else { Status::Delivered },
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
        });
        self.batch.last_mut().unwrap()
    }

    fn quote_of(&self, id: &str) -> Option<Quote> {
        self.batch.iter().find(|m| m.id == id).map(|m| m.quote())
    }

    fn react(m: &mut Message, pairs: &[(&str, &str)]) {
        for (who, emoji) in pairs {
            m.reactions.push(Reaction { emoji: emoji.to_string(), sender: who.to_string(), ts: m.ts + 30_000 });
        }
    }

    fn image(&self, seed: u32, w: u32, h: u32, caption: &str, ready: bool) -> Content {
        if !ready {
            let small = art::jpeg(&art::scene(seed, 96, 96 * h / w), 60);
            return Content::Image {
                media: Media { token: None, mime: "image/jpeg".into(), size: (w * h / 9) as u64, state: MediaState::Remote },
                caption: caption.into(),
                width: w,
                height: h,
                thumb: media::thumb_from_bytes(&small, 64),
            };
        }
        let bytes = art::jpeg(&art::scene(seed, w, h), 84);
        let token = self.app.media.write("m", "jpg", &bytes).ok();
        Content::Image {
            media: Media { token, mime: "image/jpeg".into(), size: bytes.len() as u64, state: MediaState::Ready },
            caption: caption.into(),
            width: w,
            height: h,
            thumb: media::thumb_from_bytes(&bytes, 64),
        }
    }

    fn video(&self, gif: bool, caption: &str) -> Content {
        let (bytes, w, h, dur, seed): (&[u8], u32, u32, u32, u32) = if gif {
            (include_bytes!("assets/loop.mp4"), 360, 360, 2, 2)
        } else {
            (include_bytes!("assets/clip.mp4"), 640, 360, 5, 0)
        };
        let token = self.app.media.write("m", "mp4", bytes).ok();
        let poster = art::jpeg(&art::scene(seed, 192, 192 * h / w), 60);
        Content::Video {
            media: Media { token, mime: "video/mp4".into(), size: bytes.len() as u64, state: MediaState::Ready },
            caption: caption.into(),
            width: w,
            height: h,
            duration: dur,
            gif,
            thumb: media::thumb_from_bytes(&poster, 96),
        }
    }

    fn voice(&self, seed: u32, secs: f32) -> Content {
        #[cfg(feature = "voice")]
        {
            let pcm = art::speech_pcm(seed, secs);
            if let Ok(ogg) = crate::voice::encode_ogg_opus(&pcm) {
                let token = self.app.media.write("m", "ogg", &ogg).ok();
                return Content::Audio {
                    media: Media {
                        token,
                        mime: "audio/ogg; codecs=opus".into(),
                        size: ogg.len() as u64,
                        state: MediaState::Ready,
                    },
                    duration: secs.round() as u32,
                    voice: true,
                    waveform: crate::voice::waveform(&pcm, 64),
                };
            }
        }
        let waveform = (0..64).map(|i| (30.0 + 60.0 * ((i as f32 * 0.5 + seed as f32).sin().abs())) as u8).collect();
        Content::Audio {
            media: Media { token: None, mime: "audio/ogg; codecs=opus".into(), size: 18_000, state: MediaState::Remote },
            duration: secs.round() as u32,
            voice: true,
            waveform,
        }
    }

    fn document(&self, name: &str, caption: &str) -> Content {
        let bytes = art::pdf(name.trim_end_matches(".pdf"));
        let token = self.app.media.write("m", "pdf", &bytes).ok();
        Content::Document {
            media: Media { token, mime: "application/pdf".into(), size: bytes.len() as u64, state: MediaState::Ready },
            name: name.into(),
            caption: caption.into(),
            pages: 1,
            thumb: None,
        }
    }

    fn sticker(&self, mood: u32) -> (Content, String) {
        let bytes = art::sticker(mood);
        let token = self.app.media.write("s", "webp", &bytes).unwrap_or_default();
        (
            Content::Sticker {
                media: Media {
                    token: Some(token.clone()),
                    mime: "image/webp".into(),
                    size: bytes.len() as u64,
                    state: MediaState::Ready,
                },
                width: 512,
                height: 512,
                animated: false,
            },
            token,
        )
    }
}

fn person(app: &App, id: &str, name: Option<&str>, push: &str, about: &str, avatar: Option<u32>) {
    let token = avatar.and_then(|seed| app.media.write("a", "jpg", &art::jpeg(&art::avatar(seed), 86)).ok());
    let _ = app.store.put_contact(&Contact {
        id: id.to_string(),
        name: name.map(String::from),
        push_name: Some(push.to_string()),
        about: if about.is_empty() { None } else { Some(about.to_string()) },
        avatar: token,
        is_me: id == ME,
        ..Default::default()
    });
}

fn members(list: &[(&str, Role)]) -> Vec<Participant> {
    list.iter().map(|(id, role)| Participant { id: id.to_string(), role: *role }).collect()
}

/// Builds the whole demo data set. Blocking (image generation); run off the async threads.
pub fn seed(app: &Arc<App>) {
    let now = now_ms();
    // ── people ──
    person(app, ME, None, "Sam Carter", "Probably outside 🌲", Some(1));
    person(app, MAYA, Some("Maya Okafor"), "Maya", "Somewhere between coffee and chaos ☕", Some(0));
    person(app, LEO, Some("Leo Martins"), "Leo", "Available", Some(3));
    person(app, DAD, Some("Dad"), "Robert", "", None);
    person(app, PRIYA, Some("Priya Nair"), "Priya", "Design is how it works", Some(2));
    person(app, HANNAH, None, "Weiss Dental", "Mon–Fri 08:00–18:00", None);
    person(app, TOMAS, Some("Tomás Herrera"), "Tomás", "", None);
    person(app, AIKO, Some("Aiko Tanaka"), "aiko ✿", "写真とコーヒー", Some(4));
    person(app, NOOR, Some("Noor Haddad"), "Noor", "At the climbing wall", Some(5));
    person(app, JONAS, Some("Jonas Berg"), "Jonas", "", None);
    person(app, CHLOE, Some("Chloé Laurent"), "Chloé", "Battery about to die", Some(6));
    person(app, BEN, Some("Ben Whitaker"), "Ben", "", None);
    person(app, ZARA, Some("Zara Ali"), "Zara", "Urgent calls only", Some(7));
    person(app, OLU, Some("Olu Adeyemi"), "Olu", "🎧", None);
    person(app, INES, Some("Inés Castro"), "Inés", "", None);
    person(app, STRANGER, None, "Callum", "", None);
    app.store.upsert_contact(HANNAH, |c| {
        c.verified_name = Some("Weiss Dental Studio".into());
        c.is_business = true;
    });
    app.set_me(ME, Some("Sam Carter".into()));

    // ── groups & communities ──
    use Role::*;
    let day = 86_400_000i64;
    let groups = vec![
        Group {
            id: HIKERS.into(),
            subject: "Weekend Hikers 🥾".into(),
            description: Some("Saturday trails, sunrise starts, bad puns. Check the pinned message for the meeting point.".into()),
            owner: Some(MAYA.into()),
            created_at: now - 190 * day,
            participants: members(&[
                (MAYA, Owner), (ME, Admin), (LEO, Member), (NOOR, Admin), (JONAS, Member),
                (CHLOE, Member), (AIKO, Member), (OLU, Member),
            ]),
            approval: true,
            members_can_add: false,
            pending: vec![BEN.into()],
            invite_code: Some("Hk7dQp2LmXw9TrailDemo01".into()),
            joined: true,
            ..Default::default()
        },
        Group {
            id: CRIT.into(),
            subject: "Design Crit".into(),
            description: Some("Thursday critique. Post work-in-progress, be kind, be specific.".into()),
            owner: Some(PRIYA.into()),
            created_at: now - 420 * day,
            participants: members(&[(PRIYA, Owner), (ME, Member), (ZARA, Admin), (BEN, Member), (INES, Member)]),
            members_can_add: true,
            joined: true,
            ..Default::default()
        },
        Group {
            id: FLAT.into(),
            subject: "Flat 4B".into(),
            owner: Some(TOMAS.into()),
            created_at: now - 800 * day,
            participants: members(&[(TOMAS, Owner), (ME, Admin), (CHLOE, Member), (JONAS, Member)]),
            members_can_add: true,
            joined: true,
            ..Default::default()
        },
        Group {
            id: BOOKS.into(),
            subject: "Book Club 📚".into(),
            owner: Some(NOOR.into()),
            created_at: now - 300 * day,
            participants: members(&[(NOOR, Owner), (ME, Member), (INES, Member), (ZARA, Member)]),
            members_can_add: true,
            joined: true,
            ..Default::default()
        },
        Group {
            id: MAPLE.into(),
            subject: "Maplewood Neighbours".into(),
            description: Some("Everything happening on and around Maplewood Road. Be neighbourly.".into()),
            owner: Some(ZARA.into()),
            created_at: now - 500 * day,
            participants: members(&[(ZARA, Owner), (ME, Admin), (NOOR, Member), (BEN, Member), (INES, Member), (OLU, Member)]),
            is_community: true,
            joined: true,
            size: 214,
            ..Default::default()
        },
        Group {
            id: MAPLE_NEWS.into(),
            subject: "Maplewood Neighbours".into(),
            owner: Some(ZARA.into()),
            created_at: now - 500 * day,
            participants: members(&[(ZARA, Owner), (ME, Admin), (NOOR, Member), (BEN, Member), (INES, Member), (OLU, Member)]),
            announce: true,
            is_announcement: true,
            parent: Some(MAPLE.into()),
            joined: true,
            size: 214,
            ..Default::default()
        },
        Group {
            id: MAPLE_SELL.into(),
            subject: "Buy, Sell & Borrow".into(),
            description: Some("Photos and a price, please. Mark things as gone.".into()),
            owner: Some(ZARA.into()),
            created_at: now - 480 * day,
            participants: members(&[(ZARA, Owner), (ME, Member), (BEN, Member), (OLU, Member), (INES, Member)]),
            parent: Some(MAPLE.into()),
            members_can_add: true,
            joined: true,
            size: 131,
            ..Default::default()
        },
        Group {
            id: MAPLE_GARDEN.into(),
            subject: "Garden Club 🌱".into(),
            owner: Some(NOOR.into()),
            created_at: now - 300 * day,
            participants: members(&[(NOOR, Owner), (ME, Member), (INES, Member)]),
            parent: Some(MAPLE.into()),
            members_can_add: true,
            joined: true,
            size: 37,
            ..Default::default()
        },
        Group {
            id: MAPLE_SWAP.into(),
            subject: "Book Swap".into(),
            description: Some("Leave one, take one. Shelf is by the bus stop.".into()),
            created_at: now - 200 * day,
            parent: Some(MAPLE.into()),
            size: 48,
            ..Default::default()
        },
        Group {
            id: MAPLE_EVENTS.into(),
            subject: "Street Events".into(),
            description: Some("Planning for the summer street party and the winter market.".into()),
            created_at: now - 150 * day,
            parent: Some(MAPLE.into()),
            approval: true,
            size: 112,
            ..Default::default()
        },
        Group {
            id: RUNNERS.into(),
            subject: "Northside Runners".into(),
            description: Some("All paces welcome. Tuesdays tempo, Sundays long.".into()),
            owner: Some(JONAS.into()),
            created_at: now - 260 * day,
            participants: members(&[(JONAS, Owner), (ME, Member), (LEO, Member), (CHLOE, Member)]),
            is_community: true,
            joined: true,
            size: 86,
            ..Default::default()
        },
        Group {
            id: RUNNERS_NEWS.into(),
            subject: "Northside Runners".into(),
            owner: Some(JONAS.into()),
            created_at: now - 260 * day,
            participants: members(&[(JONAS, Owner), (ME, Member), (LEO, Member), (CHLOE, Member)]),
            announce: true,
            is_announcement: true,
            parent: Some(RUNNERS.into()),
            joined: true,
            size: 86,
            ..Default::default()
        },
        Group {
            id: RUNNERS_TEMPO.into(),
            subject: "Tuesday Tempo".into(),
            owner: Some(JONAS.into()),
            created_at: now - 240 * day,
            participants: members(&[(JONAS, Owner), (ME, Member), (LEO, Member)]),
            parent: Some(RUNNERS.into()),
            members_can_add: true,
            joined: true,
            size: 29,
            ..Default::default()
        },
    ];
    for g in groups {
        let _ = app.store.put_group(&g);
    }

    let mut s = Seeder { app: app.clone(), now, n: 0, batch: Vec::with_capacity(256) };
    let h = 60i64; // minutes in an hour
    let d = 24 * h;

    // ── Maya ──
    s.add(MAYA, MAYA, 2 * d + 40, text("Are we still on for Saturday?"));
    s.add(MAYA, ME, 2 * d + 38, text("Yes! Trailhead at 8?"));
    let m = s.add(MAYA, MAYA, 2 * d + 35, text("8 works. I'll bring the good coffee ☕"));
    m.starred = true;
    Seeder::react(m, &[(ME, "❤️")]);
    let photo = s.image(0, 1200, 800, "Scouted the route last weekend. Look at this view", true);
    let photo_id = s.add(MAYA, MAYA, d + 5 * h, photo).id.clone();
    let q = s.quote_of(&photo_id);
    let m = s.add(MAYA, ME, d + 5 * h - 2, text("Okay that's unreal 😍"));
    m.quote = q;
    let v = s.voice(3, 14.0);
    s.add(MAYA, MAYA, d + 4 * h, v);
    let m = s.add(MAYA, ME, d + 4 * h - 3, text("Listening now. Yes to the ridge detour"));
    m.edited = m.ts + 60_000;
    let link_thumb = media::thumb_from_bytes(&art::jpeg(&art::scene(5, 384, 216), 80), 192);
    s.add(
        MAYA,
        MAYA,
        d + 2 * h,
        Content::Text {
            text: "This is the loop btw https://trails.example/eagle-ridge-loop".into(),
            preview: Some(LinkPreview {
                url: "https://trails.example/eagle-ridge-loop".into(),
                title: "Eagle Ridge Loop: 11.4 km, moderate".into(),
                description: "A ridge walk with two summits, a tarn and a long descent through pine forest. Allow four to five hours.".into(),
                site: "trails.example".into(),
                thumb: link_thumb,
            }),
        },
    );
    let (st, st0) = s.sticker(0);
    s.add(MAYA, ME, d + 2 * h - 4, st);
    s.add(MAYA, MAYA, 5 * h, text("*Don't* forget sunscreen this time 😅"));
    s.add(MAYA, ME, 5 * h - 2, text("_Never again._ Packing it now"));
    s.add(MAYA, MAYA, 4 * h, Content::Deleted);
    let m = s.add(MAYA, MAYA, 3 * h, text("Weather update: clear skies until Sunday evening, light wind from the west. Freezing level above the summits."));
    m.forwarded = 1;
    s.add(MAYA, ME, 3 * h - 5, text("Perfect ✅"));
    s.add(MAYA, MAYA, 12, text("See you at 8 then!"));

    // ── Weekend Hikers ──
    s.add(HIKERS, MAYA, 190 * d, system("create", Some(MAYA), &[], "Weekend Hikers 🥾"));
    s.add(HIKERS, MAYA, 190 * d - 1, system("add", Some(MAYA), &[ME, LEO, NOOR], ""));
    s.add(HIKERS, NOOR, 6 * d, system("add", Some(NOOR), &[OLU], ""));
    let pin_id = {
        let m = s.add(HIKERS, MAYA, 3 * d, text("Meet at the *north car park*, 7:45 sharp. Parking fills up by 8."));
        m.pinned_until = now + 5 * day;
        Seeder::react(m, &[(NOOR, "👍"), (LEO, "👍"), (ME, "👍"), (OLU, "🙏")]);
        m.id.clone()
    };
    s.add(HIKERS, MAYA, 3 * d - 1, system("pin", Some(MAYA), &[], &pin_id));
    s.add(
        HIKERS,
        NOOR,
        2 * d + 6 * h,
        Content::Poll {
            question: "Which trail on Saturday?".into(),
            options: vec![
                PollOption { name: "Eagle Ridge Loop".into(), voters: vec![MAYA.into(), ME.into(), LEO.into(), AIKO.into()] },
                PollOption { name: "Tarn and back (easy)".into(), voters: vec![CHLOE.into(), JONAS.into()] },
                PollOption { name: "Full traverse (long!)".into(), voters: vec![NOOR.into()] },
            ],
            selectable: 1,
        },
    );
    let album = "ALBUM-HIKE-1".to_string();
    for (i, seed) in [1u32, 3, 4].iter().enumerate() {
        let c = s.image(*seed, 1200, 900, if i == 0 { "Last month, for motivation" } else { "" }, true);
        let m = s.add(HIKERS, NOOR, 2 * d + 2 * h, c);
        m.album = Some(album.clone());
        if i == 2 {
            Seeder::react(m, &[(MAYA, "😍"), (ME, "😍"), (AIKO, "🔥"), (OLU, "🔥"), (CHLOE, "😍")]);
        }
    }
    s.add(
        HIKERS,
        LEO,
        d + 8 * h,
        Content::Location {
            lat: 54.4609,
            lng: -3.0886,
            name: "Eagle Ridge Trailhead".into(),
            address: "North car park, Valley Road".into(),
            live: false,
            thumb: None,
        },
    );
    let m = s.add(HIKERS, MAYA, d + 7 * h, text("@15550100000 can you bring the first aid kit?"));
    m.mentions = vec![ME.into()];
    let ask = m.id.clone();
    let q = s.quote_of(&ask);
    let m = s.add(HIKERS, ME, d + 7 * h - 4, text("On it 👍 I'll throw in the blister plasters too"));
    m.quote = q;
    m.status = Status::Read;
    let mine = m.id.clone();
    for (who, off) in [(MAYA, 1), (LEO, 2), (NOOR, 3), (JONAS, 9), (CHLOE, 12), (AIKO, 14), (OLU, 40)] {
        let ts = now - (d + 7 * h - 4) * 60_000;
        let _ = app.store.put_receipt(HIKERS, &mine, who, Status::Delivered, ts + off * 20_000);
        let _ = app.store.put_receipt(HIKERS, &mine, who, Status::Read, ts + off * 90_000);
    }
    s.add(
        HIKERS,
        MAYA,
        d + 3 * h,
        Content::Event {
            name: "Summit breakfast".into(),
            description: "Bring something to share. I've got the stove and the coffee.".into(),
            location: "Eagle Ridge, east summit".into(),
            start: now + 2 * day + 3 * 3_600_000,
            end: now + 2 * day + 5 * 3_600_000,
            canceled: false,
            responses: EventResponses {
                going: vec![MAYA.into(), NOOR.into(), LEO.into()],
                maybe: vec![CHLOE.into()],
                declined: vec![JONAS.into()],
            },
        },
    );
    s.add(
        HIKERS,
        CHLOE,
        20 * h,
        Content::Contacts {
            cards: vec![ContactCard {
                name: "Valley Ranger Station".into(),
                vcard: "BEGIN:VCARD\nVERSION:3.0\nFN:Valley Ranger Station\nTEL;type=WORK:+44 1632 960123\nEND:VCARD".into(),
                phone: Some("+44 1632 960123".into()),
                jid: None,
            }],
        },
    );
    let g = s.video(true, "");
    s.add(HIKERS, OLU, 9 * h, g);
    s.add(HIKERS, AIKO, 9 * h - 5, Content::Sticker {
        media: Media {
            token: app.media.write("s", "webp", include_bytes!("assets/wave.webp")).ok(),
            mime: "image/webp".into(),
            size: include_bytes!("assets/wave.webp").len() as u64,
            state: MediaState::Ready,
        },
        width: 512,
        height: 512,
        animated: true,
    });
    s.add(HIKERS, JONAS, 55, text("Forecast still looks good. I'm driving if anyone needs a lift from the station"));
    s.add(HIKERS, CHLOE, 31, text("Yes please! 🙋‍♀️ 7:10 train gets in at 7:32"));

    // ── Dad ──
    s.add(DAD, DAD, 3 * d, text("Your mother found your old walking boots in the loft. Do you still want them?"));
    s.add(DAD, ME, 3 * d - 30, text("Yes!! Don't let her give them away"));
    let garden = s.image(1, 1200, 1600, "Tomatoes are finally coming in", false);
    s.add(DAD, DAD, d + 2 * h, garden);
    s.add(DAD, DAD, 6 * h, Content::Call { video: false, missed: true, duration: 0 });
    let v = s.voice(7, 23.0);
    s.add(DAD, DAD, 6 * h - 3, v);
    s.add(DAD, DAD, 6 * h - 4, text("Call me when you're free"));
    s.add(DAD, ME, 5 * h, text("Will do, after 6")).status = Status::Delivered;

    // ── Design Crit ──
    s.add(CRIT, PRIYA, 420 * d, system("create", Some(PRIYA), &[], "Design Crit"));
    let doc = s.document("Onboarding-flow-v3.pdf", "Updated flow for Thursday. Main change is the permissions step moving *after* the first win.");
    let doc_id = s.add(CRIT, PRIYA, d + 3 * h, doc).id.clone();
    let q = s.quote_of(&doc_id);
    let m = s.add(
        CRIT,
        ZARA,
        d + 2 * h,
        text("Read it twice. Notes:\n- Step 2 asks for too much before showing any value\n- The empty state is doing a lot of work, and it works\n- Could we lose the progress dots? They make four steps feel like forty\n\n> permissions after the first win\nStrong agree with this."),
    );
    m.quote = q;
    Seeder::react(m, &[(PRIYA, "🙏"), (ME, "💯")]);
    s.add(CRIT, BEN, d + h, text("For the transition I'd try `cubic-bezier(0.2, 0, 0, 1)` at 240ms.\n```\n.sheet {\n  transition: transform 240ms cubic-bezier(0.2, 0, 0, 1);\n}\n```"));
    s.add(CRIT, ME, 22 * h, text("Agree on the dots. I mocked a version without them, will bring it Thursday"));
    s.add(CRIT, INES, 7 * h, text("Reference for the empty state: https://example.design/notes/empty-states ~not the illustration, the copy~ the pacing of it"));
    let m = s.add(CRIT, PRIYA, 2 * h, text("@15550100000 can you present first on Thursday? 10 mins"));
    m.mentions = vec![ME.into()];

    // ── Flat 4B (muted, busy) ──
    let flat_lines: [(&str, &str); 14] = [
        (TOMAS, "Bins go out tonight, it's recycling week"),
        (CHLOE, "I'll do it, I'm home early"),
        (JONAS, "Is the wifi being weird for anyone else"),
        (CHLOE, "yes!! keeps dropping"),
        (TOMAS, "Router's been restarted. Give it five"),
        (JONAS, "Back. Hero."),
        (CHLOE, "Whoever left the bike in the hallway, the landlord is coming Thursday"),
        (JONAS, "Mine, moving it tonight"),
        (TOMAS, "Also the boiler guy is Friday 9–12"),
        (CHLOE, "I can be in"),
        (TOMAS, "Legend"),
        (JONAS, "Anyone want anything from the shop"),
        (CHLOE, "Oat milk please 🙏"),
        (TOMAS, "@15550100000 did you pay the electricity?"),
    ];
    for (i, (who, line)) in flat_lines.iter().enumerate() {
        let m = s.add(FLAT, who, 4 * h - i as i64 * 9, text(line));
        if line.contains("@1555") {
            m.mentions = vec![ME.into()];
        }
    }

    // ── Leo (unread) ──
    s.add(LEO, ME, 2 * d, text("Did you manage to get tickets for the 14th?"));
    s.add(LEO, LEO, 2 * d - 20, text("Queue is 2,000 people long. Pray for me"));
    s.add(LEO, LEO, 50, text("GOT THEM"));
    s.add(LEO, LEO, 49, text("Two, standing, row doesn't matter because standing"));
    s.add(LEO, LEO, 48, text("You owe me £42 and a drink"));

    // ── Priya (disappearing messages, view once) ──
    s.add(PRIYA, PRIYA, 5 * d, system("ephemeral", Some(PRIYA), &[], "604800"));
    for (who, mins, line) in [
        (PRIYA, 5 * d - 5, "Trying disappearing messages for the salary chat, hope that's ok"),
        (ME, 5 * d - 9, "Good call"),
        (PRIYA, 26 * h, "Offer letter came through. Sending a photo, it's view once"),
    ] {
        let m = s.add(PRIYA, who, mins, text(line));
        m.expires = m.ts + 7 * day;
    }
    let once = s.image(2, 900, 1200, "", false);
    let m = s.add(PRIYA, PRIYA, 26 * h - 1, once);
    m.view_once = true;
    m.expires = m.ts + 7 * day;
    let m = s.add(PRIYA, ME, 25 * h, text("Congratulations!! 🎉 You earned every bit of that"));
    m.expires = m.ts + 7 * day;
    Seeder::react(m, &[(PRIYA, "🥹")]);

    // ── Weiss Dental (business) ──
    let m = s.add(HANNAH, HANNAH, 4 * d, text("Hello Sam, this is a reminder of your appointment with Dr. Weiss on *Tuesday at 09:30*. Reply *1* to confirm or *2* to reschedule."));
    m.starred = true;
    s.add(HANNAH, ME, 4 * d - 12, text("1"));
    s.add(HANNAH, HANNAH, 4 * d - 13, text("Thank you, your appointment is confirmed. See you on Tuesday."));

    // ── Tomás (archived) ──
    s.add(TOMAS, TOMAS, 9 * d, text("Cheers for the lift to the airport"));
    s.add(TOMAS, ME, 9 * d - 3, text("Any time. Bring back something edible"));

    // ── unknown number ──
    s.add(STRANGER, STRANGER, 28 * h, text("Hi, saw your post in Buy, Sell & Borrow. Is the bike rack still available?"));

    // ── Aiko ──
    let clip = s.video(false, "Sunset from the ferry last night");
    let clip_id = s.add(AIKO, AIKO, 2 * d, clip).id.clone();
    let q = s.quote_of(&clip_id);
    let m = s.add(AIKO, ME, 2 * d - 8, text("Stop it. That colour is illegal"));
    m.quote = q;
    let (st, st1) = s.sticker(1);
    s.add(AIKO, AIKO, 2 * d - 9, st);
    let (st, st2) = s.sticker(2);
    s.add(AIKO, ME, 2 * d - 10, st);
    let (_, st3) = s.sticker(3);
    s.add(AIKO, AIKO, 30 * h, text("来週の土曜日、空いてる？ New ramen place opened near the station 🍜"));
    s.add(AIKO, ME, 29 * h, text("Always free for ramen")).status = Status::Read;

    // ── Book Club (archived) ──
    s.add(
        BOOKS,
        NOOR,
        12 * d,
        Content::Poll {
            question: "Next month's book?".into(),
            options: vec![
                PollOption { name: "The Overstory".into(), voters: vec![NOOR.into(), ME.into()] },
                PollOption { name: "Piranesi".into(), voters: vec![INES.into(), ZARA.into(), ME.into()] },
                PollOption { name: "A Month in the Country".into(), voters: vec![] },
            ],
            selectable: 0,
        },
    );
    s.add(BOOKS, INES, 11 * d, text("Piranesi it is. Short enough that we might all actually finish it"));

    // ── notes to self ──
    s.add(ME, ME, 8 * d, text("Groceries: oat milk, lemons, coriander, the good bread"));
    s.add(ME, ME, 3 * d, text("Boots: resole or replace? Ask at the shop on Mill Lane"));
    s.add(ME, ME, 20 * h, text("Pack list\n1. Map + compass\n2. First aid kit\n3. Head torch\n4. Spare layer"));

    // ── Maplewood community ──
    let party = s.image(5, 1280, 720, "*Street party, Saturday 21st.* Road closes at noon. Bring a dish, a chair and a neighbour you haven't met yet.", true);
    let m = s.add(MAPLE_NEWS, ZARA, 2 * d, party);
    Seeder::react(m, &[(NOOR, "🎉"), (BEN, "🎉"), (ME, "❤️"), (INES, "🎉"), (OLU, "👏")]);
    s.add(MAPLE_NEWS, ZARA, 26 * h, text("Reminder: bin collection moves to *Thursday* this week because of the bank holiday."));
    s.add(
        MAPLE_NEWS,
        ZARA,
        3 * h,
        Content::Event {
            name: "Winter market planning".into(),
            description: "First meeting. We need stallholders, a rota and someone who owns a gazebo.".into(),
            location: "St Anne's hall".into(),
            start: now + 6 * day,
            end: 0,
            canceled: false,
            responses: EventResponses { going: vec![ZARA.into(), NOOR.into()], maybe: vec![BEN.into()], declined: vec![] },
        },
    );
    let rack = s.image(3, 1000, 1000, "Roof bike rack, fits most estates. £25 or swap for a decent strimmer", true);
    s.add(MAPLE_SELL, ME, 30 * h, rack).status = Status::Read;
    s.add(MAPLE_SELL, BEN, 29 * h, text("Is it the kind with the wheel trays?"));
    s.add(MAPLE_SELL, ME, 29 * h - 10, text("Yep, three trays, locks included")).status = Status::Read;
    s.add(MAPLE_SELL, OLU, 8 * h, text("Free to a good home: about forty terracotta pots. Collection only, they weigh a ton"));
    s.add(MAPLE_GARDEN, NOOR, 3 * d, text("Seed swap is back on the first Sunday. Label your envelopes this time 🙂"));
    s.add(MAPLE_GARDEN, INES, 2 * d + h, text("Does anyone know what's eating my hostas"));
    s.add(MAPLE_GARDEN, NOOR, 2 * d, text("Slugs. It's always slugs."));
    s.add(RUNNERS_NEWS, JONAS, 28 * h, text("Tuesday tempo moves to *6:30pm* from next week. Meet at the bandstand as usual."));
    s.add(RUNNERS_TEMPO, JONAS, 3 * d, text("6 × 1km at threshold tonight, 90s jog recoveries"));
    s.add(RUNNERS_TEMPO, LEO, 3 * d - 20, text("My legs have filed a complaint"));

    let batch = std::mem::take(&mut s.batch);
    app.ingest_history(batch);

    // ── chat list state ──
    app.chat_mut(MAYA, |c| c.pinned = now - 10 * day);
    app.chat_mut(HIKERS, |c| {
        c.pinned = now - 20 * day;
        c.unread = 2;
    });
    app.chat_mut(ME, |c| c.favorite = true);
    app.chat_mut(MAYA, |c| c.favorite = true);
    app.chat_mut(LEO, |c| c.unread = 3);
    app.chat_mut(CRIT, |c| {
        c.unread = 1;
        c.unread_mentions = 1;
    });
    app.chat_mut(FLAT, |c| {
        c.muted_until = -1;
        c.unread = 12;
        c.unread_mentions = 1;
    });
    app.chat_mut(PRIYA, |c| c.ephemeral = 7 * 86_400);
    app.chat_mut(TOMAS, |c| c.archived = true);
    app.chat_mut(BOOKS, |c| c.archived = true);
    app.chat_mut(STRANGER, |c| c.unread = 1);
    app.chat_mut(MAPLE_NEWS, |c| c.unread = 1);
    app.chat_mut(DAD, |c| c.draft = Some("Sorry, was in a meeting. Calling ".into()));

    // ── sticker library ──
    let library = |token: String, used: i64, favorite: bool, animated: bool| {
        let Some(bytes) = app.media.path(&token).and_then(|p| std::fs::read(p).ok()) else { return };
        let _ = app.store.put_sticker(&Sticker {
            id: file_hash(&bytes),
            token: Some(token),
            width: 512,
            height: 512,
            animated,
            favorite,
            used,
            remote: None,
        });
    };
    for (i, token) in [st0, st1, st2, st3].into_iter().enumerate() {
        library(token, now - i as i64 * 60_000, i < 2, false);
    }
    if let Ok(token) = app.media.write("s", "webp", include_bytes!("assets/wave.webp")) {
        library(token, now - 5 * 60_000, false, true);
    }

    if std::env::var("RIPPLE_DEMO_STRESS").is_ok() {
        stress(app, now);
    }
}

/// Optional load: one chat with 50,000 messages and 2,000 extra chats.
fn stress(app: &Arc<App>, now: i64) {
    let chat = "15550199999@s.whatsapp.net";
    app.store.upsert_contact(chat, |c| c.name = Some("Stress Test".into()));
    let words = ["ridge", "coffee", "tickets", "tomatoes", "router", "ferry", "ramen", "slugs", "market", "tempo"];
    let mut batch = Vec::with_capacity(5000);
    for i in 0..50_000i64 {
        let from_me = i % 3 == 0;
        batch.push(Message {
            id: format!("STRESS{i:06}"),
            chat: chat.into(),
            seq: 0,
            sender: if from_me { ME.into() } else { chat.into() },
            from_me,
            ts: now - (50_000 - i) * 45_000,
            content: text(&format!(
                "Message {i}: {} {} {}",
                words[(i % 10) as usize],
                words[((i / 7) % 10) as usize],
                "lorem ipsum ".repeat((i % 6) as usize)
            )),
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
        });
        if batch.len() == 5000 {
            app.ingest_history(std::mem::take(&mut batch));
        }
    }
    let mut batch = Vec::with_capacity(2000);
    for i in 0..2000i64 {
        let jid = format!("1555020{i:04}@s.whatsapp.net");
        app.store.upsert_contact(&jid, |c| c.name = Some(format!("Contact {i:04}")));
        batch.push(Message {
            id: format!("BULK{i:05}"),
            chat: jid.clone(),
            seq: 0,
            sender: jid,
            from_me: false,
            ts: now - 20 * 86_400_000 - i * 600_000,
            content: text(&format!("Last message in chat {i}")),
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
        });
    }
    app.ingest_history(batch);
}
