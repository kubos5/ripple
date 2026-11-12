//! The real WhatsApp backend, built on the `whatsapp-rust` library.
//!
//! [`LiveNet`] owns the linked-device session. Everything the server tells us
//! is translated (see [`convert`]) and handed to [`App`]; everything the app
//! wants done arrives as a [`NetOp`].

mod convert;
mod events;
mod ops;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::Value;
use whatsapp_rust::Jid as WaJid;
use whatsapp_rust::bot::{Bot, BotHandle, EventDelivery};
use whatsapp_rust::client::Client;
use whatsapp_rust::store::SqliteStore;

use crate::api::{NetOp, Network};
use crate::app::App;
use crate::model::*;
use convert::{jid_str, parse_jid};

pub(crate) fn err<T, E: std::fmt::Display>(r: std::result::Result<T, E>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Running {
    handle: BotHandle,
}

pub(crate) struct CallInfo {
    from: WaJid,
    creator: WaJid,
    video: bool,
    answered: bool,
}

pub(crate) struct Inner {
    app: OnceLock<Weak<App>>,
    run: tokio::sync::Mutex<Option<Running>>,
    client: RwLock<Option<Arc<Client>>>,
    /// Hidden id (`…@lid`) → phone-number JID, learned as traffic flows.
    lids: Mutex<HashMap<String, String>>,
    /// The reverse, for addressing people inside LID-addressed groups.
    pns: Mutex<HashMap<String, String>>,
    lid_groups: Mutex<HashSet<String>>,
    calls: Mutex<HashMap<String, CallInfo>>,
    /// True from pairing until the first history sync has landed.
    first_sync: AtomicBool,
    last_history: AtomicI64,
    avatars_busy: AtomicBool,
}

pub(crate) struct Addr {
    chat: WaJid,
    sender: Option<WaJid>,
    id: whatsapp_rust::MessageId,
    from_me: bool,
}

impl Addr {
    pub(crate) fn r(&self) -> Result<whatsapp_rust::MessageRef<'_>> {
        err(whatsapp_rust::MessageRef::new(&self.chat, self.id.clone(), self.sender.as_ref(), self.from_me))
    }
}

pub struct LiveNet {
    inner: Arc<Inner>,
}

impl Default for LiveNet {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveNet {
    pub fn new() -> Self {
        LiveNet {
            inner: Arc::new(Inner {
                app: OnceLock::new(),
                run: tokio::sync::Mutex::new(None),
                client: RwLock::new(None),
                lids: Mutex::new(HashMap::new()),
                pns: Mutex::new(HashMap::new()),
                lid_groups: Mutex::new(HashSet::new()),
                calls: Mutex::new(HashMap::new()),
                first_sync: AtomicBool::new(false),
                last_history: AtomicI64::new(0),
                avatars_busy: AtomicBool::new(false),
            }),
        }
    }
}

impl Inner {
    pub(crate) fn app(&self) -> Option<Arc<App>> {
        self.app.get().and_then(|w| w.upgrade())
    }

    pub(crate) fn client(&self) -> Result<Arc<Client>> {
        self.client.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| anyhow!("Not connected to WhatsApp"))
    }

    // ───────────── address handling ─────────────

    pub(crate) fn learn(&self, lid: &str, pn: &str) {
        if lid.ends_with("@lid") && pn.ends_with(USER_SERVER) {
            lock(&self.lids).insert(lid.to_string(), pn.to_string());
            lock(&self.pns).insert(pn.to_string(), lid.to_string());
        }
    }

    /// The id Ripple files a person under: their phone-number JID whenever we know it.
    pub(crate) fn canon(&self, jid: &WaJid, alt: Option<&WaJid>) -> String {
        let s = jid_str(jid);
        if !jid.is_lid() {
            if let Some(alt) = alt.filter(|a| a.is_lid()) {
                self.learn(&jid_str(alt), &s);
            }
            return s;
        }
        if let Some(alt) = alt.filter(|a| a.is_pn()) {
            let pn = jid_str(alt);
            self.learn(&s, &pn);
            return pn;
        }
        lock(&self.lids).get(&s).cloned().unwrap_or(s)
    }

    /// Like [`canon`], for the JID strings found inside protobuf messages.
    pub(crate) fn canon_s(&self, s: &str) -> String {
        let b = bare(s);
        if b.ends_with("@lid") { lock(&self.lids).get(&b).cloned().unwrap_or(b) } else { b }
    }

    /// [`canon`], falling back to the library's persisted LID table.
    pub(crate) async fn resolve(&self, client: &Client, jid: &WaJid, alt: Option<&WaJid>) -> String {
        let s = self.canon(jid, alt);
        if !s.ends_with("@lid") {
            return s;
        }
        if let Ok(Some(entry)) = client.get_lid_pn_entry(jid).await {
            let pn = entry.phone_number.to_string();
            let pn = if pn.contains('@') { bare(&pn) } else { format!("{pn}@{USER_SERVER}") };
            self.learn(&s, &pn);
            return pn;
        }
        s
    }

    /// The address to put on the wire for a member of `chat`.
    pub(crate) fn wire_member(&self, chat: &str, member: &str) -> String {
        if lock(&self.lid_groups).contains(chat)
            && let Some(lid) = lock(&self.pns).get(member) {
                return lid.clone();
            }
        member.to_string()
    }

    /// Owned addressing of a stored message, from which the library's
    /// borrowed `MessageRef` is taken for each operation.
    pub(crate) fn addr_of(&self, m: &Message) -> Result<Addr> {
        Ok(Addr {
            chat: parse_jid(&m.chat)?,
            sender: self.participant_of(m),
            id: err(whatsapp_rust::MessageId::new(&m.id))?,
            from_me: m.from_me,
        })
    }

    pub(crate) fn participant_of(&self, m: &Message) -> Option<WaJid> {
        if !is_group(&m.chat) || m.from_me {
            return None;
        }
        parse_jid(&self.wire_member(&m.chat, &m.sender)).ok()
    }

    // ───────────── session lifecycle ─────────────

    fn wa_db(app: &App) -> String {
        app.root.join("wa.db").to_string_lossy().into_owned()
    }

    /// Starts the linked-device session. With `phone`, also asks for a pairing code.
    async fn launch(self: &Arc<Self>, phone: Option<String>) -> Result<()> {
        let app = self.app().ok_or_else(|| anyhow!("app is gone"))?;
        let mut run = self.run.lock().await;
        if run.is_some() {
            return Ok(());
        }
        let backend = err(SqliteStore::open(&Self::wa_db(&app)).await)?;
        let me = self.clone();
        let mut builder = Bot::builder()
            .with_backend(backend)
            .with_event_delivery(EventDelivery::Ordered { capacity: 16_384 })
            .with_device_props(whatsapp_rust::wacore::store::device::DevicePropsOverride {
                os: Some("Ripple".into()),
                ..Default::default()
            })
            .on_event(move |ev, client| {
                let me = me.clone();
                async move { me.on_event(ev, client).await }
            });
        if let Some(number) = phone {
            builder = builder.with_pair_code(whatsapp_rust::pair_code::PairCodeOptions {
                phone_number: number,
                ..Default::default()
            });
        }
        let bot = err(builder.build().await)?;
        *self.client.write().unwrap_or_else(|e| e.into_inner()) = Some(bot.client());
        *run = Some(Running { handle: bot.spawn() });
        Ok(())
    }

    pub(crate) async fn stop(&self) {
        let running = self.run.lock().await.take();
        *self.client.write().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(r) = running {
            let _ = r.handle.shutdown().await;
        }
    }

    /// Forgets the linked session on disk so a new pairing starts clean.
    pub(crate) fn forget_session(&self, app: &App) {
        let _ = app.store.set_meta("paired", &false);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(app.root.join(format!("wa.db{suffix}")));
        }
    }
}

#[async_trait]
impl Network for LiveNet {
    async fn start(&self, app: Arc<App>) -> Result<()> {
        let _ = self.inner.app.set(Arc::downgrade(&app));
        if app.store.meta::<bool>("paired").unwrap_or(false) {
            app.set_session(Session::Connecting);
            if let Err(e) = self.inner.launch(None).await {
                log::error!("could not start the WhatsApp session: {e:#}");
                app.set_session(Session::Offline);
                app.toast("error", format!("Could not start the WhatsApp session: {e}"));
            }
        } else {
            app.set_session(Session::Welcome);
        }
        Ok(())
    }

    async fn exec(&self, op: NetOp) -> Result<Value> {
        let app = self.inner.app().ok_or_else(|| anyhow!("app is gone"))?;
        match op {
            NetOp::Pair { phone } => {
                // A stale half-paired session would make the server refuse a new link.
                if !app.store.meta::<bool>("paired").unwrap_or(false) {
                    self.inner.stop().await;
                    self.inner.forget_session(&app);
                }
                let digits = phone.map(|p| p.chars().filter(|c| c.is_ascii_digit()).collect::<String>());
                if digits.as_ref().is_some_and(|d| d.len() < 8) {
                    bail!("Enter your full phone number, including the country code");
                }
                app.set_session(Session::Pairing { qr: None, code: None, expires: 0 });
                if let Err(e) = self.inner.launch(digits).await {
                    app.set_session(Session::Welcome);
                    return Err(e);
                }
                Ok(Value::Null)
            }
            NetOp::Logout => {
                if let Ok(client) = self.inner.client() {
                    let _ = client.logout().await;
                }
                self.inner.stop().await;
                self.inner.forget_session(&app);
                Ok(Value::Null)
            }
            NetOp::DemoTrigger { .. } => bail!("Only available in the demo"),
            other => {
                let client = self.inner.client()?;
                self.inner.exec(&app, &client, other).await
            }
        }
    }

    fn is_live(&self) -> bool {
        true
    }

    async fn shutdown(&self) {
        self.inner.stop().await;
    }
}

/// Converts library group metadata into Ripple's group model.
pub(crate) fn group_from(inner: &Inner, me: &str, md: &whatsapp_rust::features::GroupMetadata) -> Group {
    use whatsapp_rust::features::{MemberAddMode, ParticipantType};
    let id = jid_str(&md.id);
    let participants: Vec<Participant> = md
        .participants
        .iter()
        .map(|p| Participant {
            id: inner.canon(&p.jid, p.phone_number.as_ref().or(p.lid.as_ref())),
            role: match p.participant_type {
                ParticipantType::SuperAdmin => Role::Owner,
                ParticipantType::Admin => Role::Admin,
                _ => Role::Member,
            },
        })
        .collect();
    if md.participants.iter().any(|p| p.jid.is_lid()) {
        lock(&inner.lid_groups).insert(id.clone());
    }
    let joined = participants.is_empty() || participants.iter().any(|p| p.id == me);
    Group {
        id,
        subject: md.subject.clone().unwrap_or_default(),
        description: md.description.clone().filter(|d| !d.is_empty()),
        owner: md.creator.as_ref().map(|c| inner.canon(c, md.creator_pn.as_ref())),
        created_at: md.creation_time.unwrap_or(0) as i64 * 1000,
        participants,
        announce: md.is_announcement,
        locked: md.is_locked,
        approval: md.membership_approval,
        members_can_add: matches!(md.member_add_mode, Some(MemberAddMode::AllMemberAdd)),
        ephemeral: md.ephemeral.as_ref().and_then(|e| e.expiration).unwrap_or(0),
        invite_code: None,
        is_community: md.is_parent_group,
        is_announcement: md.is_default_sub_group,
        parent: md.parent_group_jid.as_ref().map(jid_str),
        pending: vec![],
        joined,
        avatar: None,
        size: md.participant_count.unwrap_or(md.participants.len() as u32),
    }
}

/// Stores group metadata while keeping what only we know (avatar, invite link, requests).
pub(crate) fn merge_group(app: &App, mut g: Group) {
    if let Some(old) = app.store.group(&g.id) {
        g.avatar = g.avatar.or(old.avatar);
        g.invite_code = g.invite_code.or(old.invite_code);
        if g.pending.is_empty() {
            g.pending = old.pending;
        }
        if g.participants.is_empty() {
            g.participants = old.participants;
            g.joined = old.joined;
        }
    }
    app.ingest_group(g);
}

pub(crate) fn mark_history(inner: &Inner) {
    inner.last_history.store(now_ms(), Ordering::Relaxed);
}

pub(crate) fn first_sync(inner: &Inner) -> bool {
    inner.first_sync.load(Ordering::Relaxed)
}

pub(crate) fn avatars_busy(inner: &Inner) -> &AtomicBool {
    &inner.avatars_busy
}
