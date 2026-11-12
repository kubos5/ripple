//! The app database: chats, messages, contacts, groups and search.
//!
//! `whatsapp-rust` keeps only protocol state, so everything the user can see
//! lives here. Rows hold the JSON form of the model structs next to the few
//! columns we index on, which keeps the schema small and migrations rare.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::model::*;

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
    CREATE TABLE chats(id TEXT PRIMARY KEY, ts INTEGER NOT NULL, data TEXT NOT NULL) WITHOUT ROWID;
    CREATE TABLE contacts(id TEXT PRIMARY KEY, data TEXT NOT NULL) WITHOUT ROWID;
    CREATE TABLE groups(id TEXT PRIMARY KEY, data TEXT NOT NULL) WITHOUT ROWID;
    CREATE TABLE messages(
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        chat TEXT NOT NULL,
        id TEXT NOT NULL,
        ts INTEGER NOT NULL,
        from_me INTEGER NOT NULL,
        kind TEXT NOT NULL,
        status INTEGER NOT NULL DEFAULT 0,
        starred INTEGER NOT NULL DEFAULT 0,
        pinned_until INTEGER NOT NULL DEFAULT 0,
        expires INTEGER NOT NULL DEFAULT 0,
        data TEXT NOT NULL,
        UNIQUE(chat, id)
    );
    CREATE INDEX messages_chat_ts ON messages(chat, ts, seq);
    CREATE INDEX messages_kind ON messages(chat, kind, ts);
    CREATE INDEX messages_starred ON messages(ts) WHERE starred = 1;
    CREATE INDEX messages_pinned ON messages(chat) WHERE pinned_until > 0;
    CREATE INDEX messages_expires ON messages(expires) WHERE expires > 0;
    CREATE INDEX messages_pending ON messages(ts) WHERE status = 1 AND from_me = 1;
    CREATE VIRTUAL TABLE fts USING fts5(
        text, content='', contentless_delete=1,
        tokenize="unicode61 remove_diacritics 2"
    );
    CREATE TABLE receipts(
        chat TEXT NOT NULL, id TEXT NOT NULL, user TEXT NOT NULL,
        delivered INTEGER NOT NULL DEFAULT 0,
        read INTEGER NOT NULL DEFAULT 0,
        played INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(chat, id, user)
    ) WITHOUT ROWID;
    CREATE TABLE media(
        chat TEXT NOT NULL, id TEXT NOT NULL, params TEXT NOT NULL,
        PRIMARY KEY(chat, id)
    ) WITHOUT ROWID;
    CREATE TABLE stickers(
        token TEXT PRIMARY KEY, data TEXT NOT NULL,
        favorite INTEGER NOT NULL DEFAULT 0, used INTEGER NOT NULL DEFAULT 0
    ) WITHOUT ROWID;
    "#,
    // v2: stickers are keyed by file hash (falls back to the token for old rows)
    r#"
    ALTER TABLE stickers RENAME COLUMN token TO id;
    "#,
];

/// Position of a message in a chat's timeline.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    pub ts: i64,
    pub seq: i64,
}

fn status_code(s: Status) -> i64 {
    match s {
        Status::Failed => 0,
        Status::Pending => 1,
        Status::Sent => 2,
        Status::Delivered => 3,
        Status::Read => 4,
        Status::Played => 5,
    }
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::init(conn)
    }

    pub fn memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             PRAGMA temp_store=MEMORY; PRAGMA cache_size=-16000; PRAGMA busy_timeout=5000;",
        )?;
        conn.set_prepared_statement_cache_capacity(64);
        let version = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version={}; COMMIT;", i + 1))
                .with_context(|| format!("migration {}", i + 1))?;
        }
        Ok(Store { conn: Mutex::new(conn) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Removes every row (used when the account is unlinked). Settings survive.
    pub fn wipe(&self) -> Result<()> {
        self.db().execute_batch(
            "BEGIN;
             DELETE FROM fts; DELETE FROM messages; DELETE FROM chats; DELETE FROM contacts;
             DELETE FROM groups; DELETE FROM receipts; DELETE FROM media; DELETE FROM stickers;
             DELETE FROM meta WHERE key <> 'settings';
             COMMIT; VACUUM;",
        )?;
        Ok(())
    }

    // ───────────── meta ─────────────

    pub fn meta<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let raw: Option<String> = self
            .db()
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn set_meta<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db().execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    // ───────────── generic JSON rows ─────────────

    fn all<T: DeserializeOwned>(&self, table: &str) -> Vec<T> {
        let db = self.db();
        let Ok(mut st) = db.prepare_cached(&format!("SELECT data FROM {table}")) else {
            return vec![];
        };
        let rows = st.query_map([], |r| r.get::<_, String>(0));
        match rows {
            Ok(rows) => rows.flatten().filter_map(|s| serde_json::from_str(&s).ok()).collect(),
            Err(_) => vec![],
        }
    }

    fn one<T: DeserializeOwned>(&self, table: &str, id: &str) -> Option<T> {
        let raw: Option<String> = self
            .db()
            .prepare_cached(&format!("SELECT data FROM {table} WHERE id=?1"))
            .ok()?
            .query_row([id], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    // ───────────── chats ─────────────

    pub fn chats(&self) -> Vec<Chat> {
        self.all("chats")
    }

    pub fn chat(&self, id: &str) -> Option<Chat> {
        self.one("chats", id)
    }

    pub fn put_chat(&self, chat: &Chat) -> Result<()> {
        self.db()
            .prepare_cached(
                "INSERT INTO chats(id,ts,data) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET ts=?2, data=?3",
            )?
            .execute(params![chat.id, chat.ts, serde_json::to_string(chat)?])?;
        Ok(())
    }

    /// Applies `f` to an existing chat and stores the result.
    pub fn update_chat(&self, id: &str, f: impl FnOnce(&mut Chat)) -> Option<Chat> {
        let mut chat = self.chat(id)?;
        f(&mut chat);
        self.put_chat(&chat).ok()?;
        Some(chat)
    }

    pub fn delete_chat(&self, id: &str) -> Result<()> {
        self.clear_chat(id)?;
        self.db().execute("DELETE FROM chats WHERE id=?1", [id])?;
        Ok(())
    }

    // ───────────── contacts & groups ─────────────

    pub fn contacts(&self) -> Vec<Contact> {
        self.all("contacts")
    }

    pub fn contact(&self, id: &str) -> Option<Contact> {
        self.one("contacts", id)
    }

    pub fn put_contact(&self, c: &Contact) -> Result<()> {
        self.db()
            .prepare_cached(
                "INSERT INTO contacts(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=?2",
            )?
            .execute(params![c.id, serde_json::to_string(c)?])?;
        Ok(())
    }

    /// Creates the contact if needed, applies `f`, stores and returns it.
    pub fn upsert_contact(&self, id: &str, f: impl FnOnce(&mut Contact)) -> Contact {
        let mut c = self.contact(id).unwrap_or_else(|| Contact { id: id.to_string(), ..Default::default() });
        f(&mut c);
        let _ = self.put_contact(&c);
        c
    }

    pub fn groups(&self) -> Vec<Group> {
        self.all("groups")
    }

    pub fn group(&self, id: &str) -> Option<Group> {
        self.one("groups", id)
    }

    pub fn put_group(&self, g: &Group) -> Result<()> {
        self.db()
            .prepare_cached(
                "INSERT INTO groups(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=?2",
            )?
            .execute(params![g.id, serde_json::to_string(g)?])?;
        Ok(())
    }

    pub fn update_group(&self, id: &str, f: impl FnOnce(&mut Group)) -> Option<Group> {
        let mut g = self.group(id)?;
        f(&mut g);
        self.put_group(&g).ok()?;
        Some(g)
    }

    pub fn delete_group(&self, id: &str) -> Result<()> {
        self.db().execute("DELETE FROM groups WHERE id=?1", [id])?;
        Ok(())
    }

    // ───────────── messages ─────────────

    fn read_msg(row: &Row) -> rusqlite::Result<Option<Message>> {
        let seq: i64 = row.get(0)?;
        let data: String = row.get(1)?;
        Ok(serde_json::from_str::<Message>(&data).ok().map(|mut m| {
            m.seq = seq;
            m
        }))
    }

    fn insert_in(conn: &Connection, m: &mut Message) -> Result<bool> {
        let data = serde_json::to_string(m)?;
        let n = conn
            .prepare_cached(
                "INSERT OR IGNORE INTO messages(chat,id,ts,from_me,kind,status,starred,pinned_until,expires,data)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?
            .execute(params![
                m.chat,
                m.id,
                m.ts,
                m.from_me,
                m.content.kind(),
                status_code(m.status),
                m.starred,
                m.pinned_until,
                m.expires,
                data
            ])?;
        if n == 0 {
            return Ok(false);
        }
        m.seq = conn.last_insert_rowid();
        let text = m.content.text();
        if !text.is_empty() && !m.is_system() {
            conn.prepare_cached("INSERT INTO fts(rowid,text) VALUES(?1,?2)")?
                .execute(params![m.seq, text])?;
        }
        Ok(true)
    }

    /// Inserts a message, assigning its `seq`. Returns `false` when a message
    /// with the same id already exists in the chat (nothing is changed).
    pub fn insert_message(&self, m: &mut Message) -> Result<bool> {
        Self::insert_in(&self.db(), m)
    }

    /// Bulk insert inside one transaction (history sync).
    pub fn insert_messages(&self, msgs: &mut [Message]) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut added = 0;
        for m in msgs.iter_mut() {
            if Self::insert_in(&tx, m)? {
                added += 1;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn message(&self, chat: &str, id: &str) -> Option<Message> {
        self.db()
            .prepare_cached("SELECT seq,data FROM messages WHERE chat=?1 AND id=?2")
            .ok()?
            .query_row([chat, id], Self::read_msg)
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    /// Finds a message by id when the chat is unknown (receipts for our own messages).
    pub fn message_by_id(&self, id: &str) -> Option<Message> {
        self.db()
            .prepare_cached("SELECT seq,data FROM messages WHERE id=?1 LIMIT 1")
            .ok()?
            .query_row([id], Self::read_msg)
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    /// Read-modify-write of a stored message. Keeps indexed columns and the
    /// search index in step with the JSON.
    pub fn update_message(&self, chat: &str, id: &str, f: impl FnOnce(&mut Message)) -> Option<Message> {
        let mut db = self.db();
        let tx = db.transaction().ok()?;
        let mut m = tx
            .prepare_cached("SELECT seq,data FROM messages WHERE chat=?1 AND id=?2")
            .ok()?
            .query_row([chat, id], Self::read_msg)
            .optional()
            .ok()
            .flatten()
            .flatten()?;
        let old_text = m.content.text().to_string();
        f(&mut m);
        tx.prepare_cached(
            "UPDATE messages SET ts=?1,kind=?2,status=?3,starred=?4,pinned_until=?5,expires=?6,data=?7 WHERE seq=?8",
        )
        .ok()?
        .execute(params![
            m.ts,
            m.content.kind(),
            status_code(m.status),
            m.starred,
            m.pinned_until,
            m.expires,
            serde_json::to_string(&m).ok()?,
            m.seq
        ])
        .ok()?;
        let text = m.content.text();
        if text != old_text {
            if !old_text.is_empty() {
                tx.execute("DELETE FROM fts WHERE rowid=?1", [m.seq]).ok()?;
            }
            if !text.is_empty() && !m.is_system() {
                tx.execute("INSERT INTO fts(rowid,text) VALUES(?1,?2)", params![m.seq, text]).ok()?;
            }
        }
        tx.commit().ok()?;
        Some(m)
    }

    fn query_msgs(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Vec<Message> {
        let db = self.db();
        let Ok(mut st) = db.prepare_cached(sql) else { return vec![] };
        match st.query_map(args, Self::read_msg) {
            Ok(rows) => rows.flatten().flatten().collect(),
            Err(_) => vec![],
        }
    }

    /// A page of messages older than `before` (or the newest page), oldest first.
    pub fn messages(&self, chat: &str, before: Option<Cursor>, limit: usize) -> Vec<Message> {
        let c = before.unwrap_or(Cursor { ts: i64::MAX, seq: i64::MAX });
        let mut v = self.query_msgs(
            "SELECT seq,data FROM messages WHERE chat=?1 AND (ts,seq) < (?2,?3)
             ORDER BY ts DESC, seq DESC LIMIT ?4",
            &[&chat, &c.ts, &c.seq, &(limit as i64)],
        );
        v.reverse();
        v
    }

    /// A page of messages newer than `after`, oldest first.
    pub fn messages_after(&self, chat: &str, after: Cursor, limit: usize) -> Vec<Message> {
        self.query_msgs(
            "SELECT seq,data FROM messages WHERE chat=?1 AND (ts,seq) > (?2,?3)
             ORDER BY ts ASC, seq ASC LIMIT ?4",
            &[&chat, &after.ts, &after.seq, &(limit as i64)],
        )
    }

    /// `half` messages either side of the given one (jump to a reply/search hit).
    pub fn messages_around(&self, chat: &str, id: &str, half: usize) -> Vec<Message> {
        let Some(m) = self.message(chat, id) else { return vec![] };
        let at = Cursor { ts: m.ts, seq: m.seq };
        let mut v = self.messages(chat, Some(at), half);
        v.push(m);
        v.extend(self.messages_after(chat, at, half));
        v
    }

    pub fn last_message(&self, chat: &str) -> Option<Message> {
        self.messages(chat, None, 1).pop()
    }

    pub fn delete_message(&self, chat: &str, id: &str) -> Result<()> {
        let db = self.db();
        let seq: Option<i64> = db
            .query_row("SELECT seq FROM messages WHERE chat=?1 AND id=?2", [chat, id], |r| r.get(0))
            .optional()?;
        if let Some(seq) = seq {
            db.execute("DELETE FROM fts WHERE rowid=?1", [seq])?;
            db.execute("DELETE FROM messages WHERE seq=?1", [seq])?;
            db.execute("DELETE FROM receipts WHERE chat=?1 AND id=?2", [chat, id])?;
            db.execute("DELETE FROM media WHERE chat=?1 AND id=?2", [chat, id])?;
        }
        Ok(())
    }

    pub fn clear_chat(&self, chat: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM fts WHERE rowid IN (SELECT seq FROM messages WHERE chat=?1)", [chat])?;
        tx.execute("DELETE FROM messages WHERE chat=?1", [chat])?;
        tx.execute("DELETE FROM receipts WHERE chat=?1", [chat])?;
        tx.execute("DELETE FROM media WHERE chat=?1", [chat])?;
        tx.commit()?;
        Ok(())
    }

    pub fn starred(&self, chat: Option<&str>, limit: usize) -> Vec<Message> {
        match chat {
            Some(c) => self.query_msgs(
                "SELECT seq,data FROM messages WHERE starred=1 AND chat=?1 ORDER BY ts DESC LIMIT ?2",
                &[&c, &(limit as i64)],
            ),
            None => self.query_msgs(
                "SELECT seq,data FROM messages WHERE starred=1 ORDER BY ts DESC LIMIT ?1",
                &[&(limit as i64)],
            ),
        }
    }

    /// Messages currently pinned in a chat, newest first.
    pub fn pinned(&self, chat: &str, now: i64) -> Vec<Message> {
        self.query_msgs(
            "SELECT seq,data FROM messages WHERE chat=?1 AND pinned_until>?2 ORDER BY ts DESC",
            &[&chat, &now],
        )
    }

    /// Media gallery: `kinds` is a list such as `["image","video","gif"]`.
    pub fn gallery(&self, chat: &str, kinds: &[&str], before: Option<Cursor>, limit: usize) -> Vec<Message> {
        let c = before.unwrap_or(Cursor { ts: i64::MAX, seq: i64::MAX });
        let list = kinds.iter().map(|k| format!("'{}'", k.replace('\'', ""))).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT seq,data FROM messages WHERE chat=?1 AND kind IN ({list}) AND (ts,seq) < (?2,?3)
             ORDER BY ts DESC, seq DESC LIMIT ?4"
        );
        self.query_msgs(&sql, &[&chat, &c.ts, &c.seq, &(limit as i64)])
    }

    /// Text messages containing a link, newest first.
    pub fn links(&self, chat: &str, limit: usize) -> Vec<Message> {
        self.query_msgs(
            "SELECT seq,data FROM messages WHERE chat=?1 AND kind='text'
             AND (data LIKE '%http://%' OR data LIKE '%https://%') ORDER BY ts DESC LIMIT ?2",
            &[&chat, &(limit as i64)],
        )
    }

    /// Full-text search, newest first. Every word must match; the last word matches as a prefix.
    pub fn search(&self, query: &str, chat: Option<&str>, limit: usize) -> Vec<Message> {
        let Some(q) = fts_query(query) else { return vec![] };
        // Walking the index by descending rowid lets SQLite stop after `limit`
        // hits instead of sorting every match; rowids follow arrival order, so
        // a final sort by timestamp only has to fix up back-filled history.
        let mut hits = match chat {
            Some(c) => self.query_msgs(
                "SELECT m.seq,m.data FROM fts JOIN messages m ON m.seq=fts.rowid
                 WHERE fts MATCH ?1 AND m.chat=?2 ORDER BY fts.rowid DESC LIMIT ?3",
                &[&q, &c, &(limit as i64)],
            ),
            None => self.query_msgs(
                "SELECT m.seq,m.data FROM fts JOIN messages m ON m.seq=fts.rowid
                 WHERE fts MATCH ?1 ORDER BY fts.rowid DESC LIMIT ?2",
                &[&q, &(limit as i64)],
            ),
        };
        hits.sort_by_key(|m| std::cmp::Reverse((m.ts, m.seq)));
        hits
    }

    /// Outgoing messages that never left (resent after reconnecting).
    pub fn pending(&self, since: i64) -> Vec<Message> {
        self.query_msgs(
            "SELECT seq,data FROM messages WHERE status=1 AND from_me=1 AND ts>?1 ORDER BY ts ASC",
            &[&since],
        )
    }

    /// Disappearing messages whose time is up.
    pub fn expired(&self, now: i64) -> Vec<(String, String)> {
        let db = self.db();
        let Ok(mut st) = db.prepare_cached("SELECT chat,id FROM messages WHERE expires>0 AND expires<=?1") else {
            return vec![];
        };
        st.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Incoming messages newer than `after` (used to recount unread).
    pub fn count_incoming_after(&self, chat: &str, after_ts: i64) -> u32 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE chat=?1 AND from_me=0 AND ts>?2
                 AND kind NOT IN ('system','call')",
                params![chat, after_ts],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    pub fn message_count(&self) -> u64 {
        self.db().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64
    }

    // ───────────── receipts ─────────────

    pub fn put_receipt(&self, chat: &str, id: &str, user: &str, status: Status, ts: i64) -> Result<()> {
        let col = match status {
            Status::Delivered => "delivered",
            Status::Read => "read",
            Status::Played => "played",
            _ => return Ok(()),
        };
        self.db().execute(
            &format!(
                "INSERT INTO receipts(chat,id,user,{col}) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(chat,id,user) DO UPDATE SET {col}=?4"
            ),
            params![chat, id, user, ts],
        )?;
        Ok(())
    }

    pub fn receipts(&self, chat: &str, id: &str) -> Vec<Receipt> {
        let db = self.db();
        let Ok(mut st) =
            db.prepare_cached("SELECT user,delivered,read,played FROM receipts WHERE chat=?1 AND id=?2")
        else {
            return vec![];
        };
        st.query_map([chat, id], |r| {
            Ok(Receipt { user: r.get(0)?, delivered: r.get(1)?, read: r.get(2)?, played: r.get(3)? })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    // ───────────── private media parameters ─────────────

    pub fn set_media_params<T: Serialize>(&self, chat: &str, id: &str, p: &T) -> Result<()> {
        self.db().execute(
            "INSERT INTO media(chat,id,params) VALUES(?1,?2,?3)
             ON CONFLICT(chat,id) DO UPDATE SET params=?3",
            params![chat, id, serde_json::to_string(p)?],
        )?;
        Ok(())
    }

    pub fn media_params<T: DeserializeOwned>(&self, chat: &str, id: &str) -> Option<T> {
        let raw: Option<String> = self
            .db()
            .query_row("SELECT params FROM media WHERE chat=?1 AND id=?2", [chat, id], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    // ───────────── stickers ─────────────

    fn read_stickers(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Vec<Sticker> {
        let db = self.db();
        let Ok(mut st) = db.prepare_cached(sql) else { return vec![] };
        st.query_map(args, |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, i64>(3)?))
        })
        .map(|rows| {
            rows.flatten()
                .filter_map(|(id, d, fav, used)| {
                    let mut s: Sticker = serde_json::from_str(&d).ok()?;
                    s.id = id;
                    s.favorite = fav;
                    s.used = used;
                    Some(s)
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Favourites plus the most recently sent stickers, newest first.
    pub fn stickers(&self) -> Vec<Sticker> {
        self.read_stickers(
            "SELECT id,data,favorite,used FROM stickers WHERE favorite=1 OR used>0 ORDER BY used DESC LIMIT 400",
            &[],
        )
    }

    /// Sticker messages the user sent, newest first: `(chat, id, ts)`.
    pub fn sent_stickers(&self, limit: usize) -> Vec<(String, String, i64)> {
        let db = self.db();
        let Ok(mut st) =
            db.prepare_cached("SELECT chat,id,ts FROM messages WHERE kind='sticker' AND from_me=1 ORDER BY ts DESC LIMIT ?1")
        else {
            return vec![];
        };
        st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn sticker(&self, id: &str) -> Option<Sticker> {
        self.read_stickers("SELECT id,data,favorite,used FROM stickers WHERE id=?1", &[&id]).pop()
    }

    pub fn sticker_by_token(&self, token: &str) -> Option<Sticker> {
        self.read_stickers(
            "SELECT id,data,favorite,used FROM stickers WHERE id=?1 OR json_extract(data,'$.token')=?1 LIMIT 1",
            &[&token],
        )
        .pop()
    }

    /// Inserts or replaces a sticker row, including its flags.
    pub fn put_sticker(&self, s: &Sticker) -> Result<()> {
        self.db().execute(
            "INSERT INTO stickers(id,data,favorite,used) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET data=?2, favorite=?3, used=?4",
            params![s.id, serde_json::to_string(s)?, s.favorite, s.used],
        )?;
        Ok(())
    }

    /// Applies `f` to the sticker with this id (a blank one when unknown) and stores it.
    pub fn update_sticker(&self, id: &str, f: impl FnOnce(&mut Sticker)) -> Result<Sticker> {
        let mut s = self.sticker(id).unwrap_or_else(|| Sticker { id: id.to_string(), width: 512, height: 512, ..Default::default() });
        f(&mut s);
        self.put_sticker(&s)?;
        Ok(s)
    }

    pub fn remove_sticker(&self, id: &str) -> Result<()> {
        self.db().execute("DELETE FROM stickers WHERE id=?1", [id])?;
        Ok(())
    }
}

/// Turns free text into a safe FTS5 query: each word quoted, last word a prefix.
fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| w.chars().filter(|c| *c != '"').collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let last = words.len() - 1;
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| if i == last { format!("\"{w}\"*") } else { format!("\"{w}\"") })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(chat: &str, id: &str, ts: i64, text: &str) -> Message {
        Message {
            id: id.into(),
            chat: chat.into(),
            seq: 0,
            sender: "1@s.whatsapp.net".into(),
            from_me: false,
            ts,
            content: Content::Text { text: text.into(), preview: None },
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

    #[test]
    fn insert_page_and_dedupe() {
        let s = Store::memory().unwrap();
        for i in 0..120 {
            let mut m = msg("c", &format!("m{i}"), 1000 + i, &format!("hello {i}"));
            assert!(s.insert_message(&mut m).unwrap());
            assert!(m.seq > 0);
        }
        let mut dup = msg("c", "m5", 1, "dup");
        assert!(!s.insert_message(&mut dup).unwrap());

        let page = s.messages("c", None, 50);
        assert_eq!(page.len(), 50);
        assert_eq!(page.first().unwrap().id, "m70");
        assert_eq!(page.last().unwrap().id, "m119");

        let first = page.first().unwrap();
        let older = s.messages("c", Some(Cursor { ts: first.ts, seq: first.seq }), 50);
        assert_eq!(older.last().unwrap().id, "m69");
        assert_eq!(older.len(), 50);

        let around = s.messages_around("c", "m60", 5);
        assert_eq!(around.len(), 11);
        assert_eq!(around[5].id, "m60");
    }

    #[test]
    fn search_update_and_delete() {
        let s = Store::memory().unwrap();
        s.insert_message(&mut msg("a", "1", 10, "Dinner at the harbour tonight")).unwrap();
        s.insert_message(&mut msg("b", "2", 20, "harbour lights café")).unwrap();
        assert_eq!(s.search("harb", None, 10).len(), 2);
        assert_eq!(s.search("harbour tonight", None, 10).len(), 1);
        assert_eq!(s.search("cafe", None, 10).len(), 1, "diacritics are folded");
        assert_eq!(s.search("harbour", Some("a"), 10).len(), 1);
        assert!(s.search("\"", None, 10).is_empty());

        s.update_message("a", "1", |m| m.content.set_text("Lunch instead".into())).unwrap();
        assert_eq!(s.search("dinner", None, 10).len(), 0);
        assert_eq!(s.search("lunch", None, 10).len(), 1);

        s.delete_message("b", "2").unwrap();
        assert_eq!(s.search("harbour", None, 10).len(), 0);
        assert!(s.message("b", "2").is_none());
    }

    #[test]
    fn flags_and_indexes_follow_updates() {
        let s = Store::memory().unwrap();
        s.insert_message(&mut msg("a", "1", 10, "x")).unwrap();
        s.update_message("a", "1", |m| {
            m.starred = true;
            m.pinned_until = 5000;
            m.expires = 100;
        });
        assert_eq!(s.starred(None, 10).len(), 1);
        assert_eq!(s.pinned("a", 1000).len(), 1);
        assert_eq!(s.pinned("a", 6000).len(), 0);
        assert_eq!(s.expired(200), vec![("a".to_string(), "1".to_string())]);
        assert!(s.expired(50).is_empty());
    }

    #[test]
    fn chats_contacts_meta() {
        let s = Store::memory().unwrap();
        s.put_chat(&Chat { id: "a".into(), name: "A".into(), ts: 5, ..Default::default() }).unwrap();
        s.update_chat("a", |c| c.unread = 3).unwrap();
        assert_eq!(s.chat("a").unwrap().unread, 3);
        assert_eq!(s.chats().len(), 1);

        let c = s.upsert_contact("1@s.whatsapp.net", |c| c.push_name = Some("Ana".into()));
        assert_eq!(c.display(), "Ana");
        s.set_meta("settings", &Settings::default()).unwrap();
        assert_eq!(s.meta::<Settings>("settings").unwrap().accent, "mono");

        s.delete_chat("a").unwrap();
        assert!(s.chat("a").is_none());
    }

    #[test]
    fn wipe_keeps_only_settings() {
        let s = Store::memory().unwrap();
        s.insert_message(&mut msg("a", "1", 10, "secret plans")).unwrap();
        s.put_chat(&Chat { id: "a".into(), ..Default::default() }).unwrap();
        s.set_meta("settings", &Settings { accent: "coral".into(), ..Default::default() }).unwrap();
        s.set_meta("me", &"1@s.whatsapp.net").unwrap();
        s.wipe().unwrap();
        assert_eq!(s.message_count(), 0);
        assert!(s.chats().is_empty() && s.search("secret", None, 5).is_empty());
        assert!(s.meta::<String>("me").is_none());
        assert_eq!(s.meta::<Settings>("settings").unwrap().accent, "coral");
    }

    #[test]
    fn sticker_library() {
        let s = Store::memory().unwrap();
        s.update_sticker("hashA", |x| {
            x.token = Some("s/a.webp".into());
            x.used = 50;
        })
        .unwrap();
        s.update_sticker("hashB", |x| x.favorite = true).unwrap();
        s.update_sticker("hashC", |x| x.token = Some("s/c.webp".into())).unwrap();
        let list = s.stickers();
        assert_eq!(list.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["hashA", "hashB"], "unused, unfavourited stickers stay hidden");
        assert_eq!(s.sticker_by_token("s/a.webp").unwrap().id, "hashA");
        let b = s.update_sticker("hashB", |x| x.used = 99).unwrap();
        assert!(b.favorite, "updating keeps flags");
        assert_eq!(s.stickers()[0].id, "hashB");
        s.remove_sticker("hashA").unwrap();
        assert!(s.sticker("hashA").is_none());
    }

    #[test]
    fn receipts_accumulate() {
        let s = Store::memory().unwrap();
        s.put_receipt("g", "1", "u1", Status::Delivered, 10).unwrap();
        s.put_receipt("g", "1", "u1", Status::Read, 20).unwrap();
        s.put_receipt("g", "1", "u2", Status::Delivered, 11).unwrap();
        let r = s.receipts("g", "1");
        assert_eq!(r.len(), 2);
        let u1 = r.iter().find(|x| x.user == "u1").unwrap();
        assert_eq!((u1.delivered, u1.read), (10, 20));
    }

    #[test]
    fn reopening_keeps_data_and_schema_version() {
        let dir = std::env::temp_dir().join(format!("ripple-store-test-{}", now_ms()));
        let path = dir.join("app.db");
        {
            let s = Store::open(&path).unwrap();
            s.insert_message(&mut msg("a", "1", 10, "persist me")).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.message_count(), 1);
        assert_eq!(s.search("persist", None, 5).len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
