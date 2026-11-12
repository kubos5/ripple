// The UI's single source of truth: a Solid store fed by `init` and by the
// event stream from the core, plus the actions components call.

import { batch, createMemo, createRoot, createSignal } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { call, listen } from "./ipc";
import { isGroupJid, phone, userPart } from "./format";
import { plain } from "./rich";
import type {
  Chat, Contact, Cursor, Group, InitData, Jid, Message, Outgoing, Privacy, Session, Settings, UiEvent,
} from "./types";

export type View = "chats" | "communities" | "starred" | "archived";
export type Filter = "all" | "unread" | "favorites" | "groups";
export type Panel = null | "info" | "search" | "starred";

export interface Toast {
  id: number;
  tone: "info" | "error" | "success";
  text: string;
}

export interface ModalState {
  type: string;
  props?: Record<string, unknown>;
}

interface Convo {
  chat: Jid | null;
  messages: Message[];
  pinned: Message[];
  loading: boolean;
  /** No more history to load above. */
  atStart: boolean;
  /** The loaded window does not include the newest messages (after a jump). */
  detached: boolean;
  /** Id of the first unread message when the chat was opened. */
  unreadFrom: string | null;
  unreadCount: number;
  /** Message to flash after a jump. */
  flash: string | null;
}

interface State {
  ready: boolean;
  bootError: string | null;
  demo: boolean;
  platform: string;
  version: string;
  session: Session;
  me: Contact | null;
  settings: Settings;
  privacy: Privacy;
  chats: Record<Jid, Chat>;
  contacts: Record<Jid, Contact>;
  groups: Record<Jid, Group>;
  view: View;
  filter: Filter;
  query: string;
  community: Jid | null;
  convo: Convo;
  typing: Record<Jid, Record<Jid, string>>;
  presence: Record<Jid, { online: boolean; lastSeen: number }>;
  reply: Message | null;
  editing: Message | null;
  selecting: boolean;
  selected: Record<string, true>;
  panel: Panel;
  modal: ModalState | null;
  lightbox: { chat: Jid; id: string } | null;
  toasts: Toast[];
  call: { id: string; from: Jid; video: boolean } | null;
  recording: { elapsed: number; levels: number[] } | null;
  badge: number;
  now: number;
  focused: boolean;
}

const DEFAULT_SETTINGS: Settings = {
  theme: "system", accent: "mono", font: "system", design: 2, textScale: 1, compact: false, enterSends: true, notifications: true,
  notificationPreview: true, notificationSound: true, autoImages: true, autoAudio: true, autoVideo: false,
  autoDocuments: false, linkPreviews: true, closeToTray: true, autostart: false, spellcheck: true,
  reduceMotion: "system", sidebarWidth: 348, quickReactions: ["👍", "❤️", "😂", "😮", "😢", "🙏"],
};

const emptyConvo = (): Convo => ({
  chat: null, messages: [], pinned: [], loading: false, atStart: false, detached: false, unreadFrom: null,
  unreadCount: 0, flash: null,
});

export const [state, setState] = createStore<State>({
  ready: false,
  bootError: null,
  demo: false,
  platform: "macos",
  version: "",
  session: { state: "welcome" },
  me: null,
  settings: DEFAULT_SETTINGS,
  privacy: { lastSeen: "contacts", online: "all", profilePhoto: "contacts", about: "contacts", readReceipts: true, groupAdd: "all", defaultEphemeral: 0 },
  chats: {},
  contacts: {},
  groups: {},
  view: "chats",
  filter: "all",
  query: "",
  community: null,
  convo: emptyConvo(),
  typing: {},
  presence: {},
  reply: null,
  editing: null,
  selecting: false,
  selected: {},
  panel: null,
  modal: null,
  lightbox: null,
  toasts: [],
  call: null,
  recording: null,
  badge: 0,
  now: Date.now(),
  focused: true,
});

/** Ids of messages that just arrived, so their bubbles animate in once. */
export const freshIds = new Set<string>();
/** Bumped whenever the list should stick to the bottom after an update. */
export const [stickSignal, setStick] = createSignal(0);
/** Bumped when the sticker library changes (sync, download, favourite). */
export const [stickerRev, setStickerRev] = createSignal(0);
/** Fired with a message id the list should scroll to. */
export const [jumpSignal, setJump] = createSignal<{ id: string; n: number } | null>(null);

// ───────────────────────────── names ─────────────────────────────

export function contactName(jid: Jid | null | undefined, opts: { short?: boolean; you?: boolean } = {}): string {
  if (!jid) return "";
  if (opts.you !== false && state.me && jid === state.me.id) return "You";
  const c = state.contacts[jid];
  const full = c?.name || c?.verifiedName || (c?.pushName ? `~${c.pushName}` : "") || phone(jid);
  if (opts.short && c?.name) return c.name.split(" ")[0];
  return full;
}

export function chatName(id: Jid): string {
  if (state.me && id === state.me.id) return `${state.me.pushName ?? "Me"} (You)`;
  if (isGroupJid(id)) return state.groups[id]?.subject || state.chats[id]?.name || "Group";
  return contactName(id, { you: false });
}

export function avatarOf(id: Jid): string | null {
  return (isGroupJid(id) ? state.groups[id]?.avatar : state.contacts[id]?.avatar) ?? state.chats[id]?.avatar ?? null;
}

const mentionName = (user: string) => {
  const jid = Object.keys(state.contacts).find((j) => userPart(j) === user);
  return jid ? contactName(jid, { you: false }).replace(/^~/, "") : undefined;
};

export function previewText(text: string) {
  return plain(text, mentionName);
}

/** One-line description of a message, for search results, pins and reply bars. */
export function messageLabel(m: Message): string {
  const c = m.content;
  switch (c.type) {
    case "text": return previewText(c.text);
    case "image": return c.caption ? previewText(c.caption) : "Photo";
    case "video": return c.caption ? previewText(c.caption) : c.gif ? "GIF" : "Video";
    case "document": return c.caption ? previewText(c.caption) : c.name;
    case "audio": return c.voice ? "Voice message" : "Audio";
    case "sticker": return "Sticker";
    case "location": return c.name || (c.live ? "Live location" : "Location");
    case "contacts": return c.cards.length === 1 ? c.cards[0].name : `${c.cards.length} contacts`;
    case "poll": return c.question;
    case "event": return c.name;
    case "call": return c.missed ? "Missed call" : "Call";
    case "deleted": return "This message was deleted";
    default: return "Message";
  }
}

export function isMuted(chat: Chat) {
  return chat.mutedUntil === -1 || chat.mutedUntil > state.now;
}

export function myRole(groupId: Jid) {
  const me = state.me?.id;
  return state.groups[groupId]?.participants.find((p) => p.id === me)?.role ?? null;
}

export function isAdmin(groupId: Jid) {
  const r = myRole(groupId);
  return r === "admin" || r === "owner";
}

// ───────────────────────────── derived lists ─────────────────────────────

const byOrder = (a: Chat, b: Chat) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || (a.pinned && b.pinned ? b.pinned - a.pinned : 0) || b.ts - a.ts;

export const derived = createRoot(() => {
  const all = createMemo(() => Object.values(state.chats));
  // A one-to-one chat with nothing in it (history sync lists some) stays out of
  // the list until it has a message, a draft, or is the chat being viewed.
  const shown = (c: Chat) => c.kind !== "direct" || !!c.last || !!c.draft || c.unread > 0 || c.id === state.convo.chat;
  const inbox = createMemo(() => all().filter((c) => !c.archived && shown(c)).sort(byOrder));
  const archived = createMemo(() => all().filter((c) => c.archived).sort((a, b) => b.ts - a.ts));
  const communities = createMemo(() =>
    Object.values(state.groups).filter((g) => g.isCommunity).sort((a, b) => a.subject.localeCompare(b.subject)),
  );
  const unreadChats = createMemo(() => inbox().filter((c) => c.unread > 0 || c.markedUnread).length);
  const archivedUnread = createMemo(() => archived().filter((c) => c.unread > 0 || c.markedUnread).length);
  const communityUnread = createMemo(() =>
    all().filter((c) => c.community && !c.archived && (c.unread > 0 || c.markedUnread)).length,
  );
  const activeChat = createMemo(() => (state.convo.chat ? state.chats[state.convo.chat] : undefined));
  return { all, inbox, archived, communities, unreadChats, archivedUnread, communityUnread, activeChat };
});

// ───────────────────────────── toasts & errors ─────────────────────────────

let toastId = 1;
export function toast(text: string, tone: Toast["tone"] = "info") {
  const id = toastId++;
  setState("toasts", (t) => [...t.slice(-2), { id, tone, text }]);
  setTimeout(() => setState("toasts", (t) => t.filter((x) => x.id !== id)), tone === "error" ? 5200 : 3200);
}

/** Runs a command; failures become an error toast instead of an exception. */
export async function act<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T | undefined> {
  try {
    return await call<T>(cmd, args);
  } catch (e) {
    toast((e as Error).message, "error");
    return undefined;
  }
}

// ───────────────────────────── boot & events ─────────────────────────────

const index = <T extends { id: string }>(list: T[]) => Object.fromEntries(list.map((x) => [x.id, x]));

async function loadInit() {
  const init = await call<InitData>("init");
  batch(() => {
    setState({
      ready: true,
      bootError: null,
      demo: init.demo,
      platform: init.platform,
      version: init.version,
      session: init.session,
      me: init.me,
      settings: { ...DEFAULT_SETTINGS, ...init.settings },
      privacy: init.privacy,
    });
    setState("chats", reconcile(index(init.chats)));
    setState("contacts", reconcile(index(init.contacts)));
    setState("groups", reconcile(index(init.groups)));
  });
}

export async function boot() {
  listen((events) => batch(() => events.forEach(apply)));
  setInterval(() => setState("now", Date.now()), 30_000);
  const focus = (on: boolean) => {
    setState("focused", on);
    void call("focus", { on }).catch(() => {});
  };
  window.addEventListener("focus", () => focus(true));
  window.addEventListener("blur", () => focus(false));
  try {
    await loadInit();
  } catch (e) {
    setState("bootError", (e as Error).message);
  }
}

function resetAll() {
  batch(() => {
    setState({
      convo: emptyConvo(), reply: null, editing: null, selecting: false, panel: null, modal: null, lightbox: null,
      typing: {}, presence: {}, view: "chats", filter: "all", query: "", community: null, call: null,
    });
    setState("selected", reconcile({}));
  });
  void loadInit().catch((e) => setState("bootError", (e as Error).message));
}

const typingTimers = new Map<string, ReturnType<typeof setTimeout>>();

function apply(ev: UiEvent) {
  switch (ev.type) {
    case "reset":
      return resetAll();
    case "session":
      return setState("session", reconcile(ev.session));
    case "chat":
      return setState("chats", ev.chat.id, reconcile(ev.chat));
    case "chatRemoved":
      setState("chats", produce((c) => void delete c[ev.id]));
      if (state.convo.chat === ev.id) closeChat();
      return;
    case "message":
      return onMessage(ev.message, ev.fresh);
    case "messageRemoved":
      if (state.convo.chat === ev.chat) {
        setState("convo", "messages", (l) => l.filter((m) => m.id !== ev.id));
        setState("convo", "pinned", (l) => l.filter((m) => m.id !== ev.id));
      }
      return;
    case "history":
      if (state.convo.chat === ev.chat && !state.convo.detached) void reloadHistory(ev.chat);
      return;
    case "contact":
      return setState("contacts", ev.contact.id, reconcile(ev.contact));
    case "me":
      setState("me", reconcile(ev.contact));
      return setState("contacts", ev.contact.id, reconcile(ev.contact));
    case "group":
      return setState("groups", ev.group.id, reconcile(ev.group));
    case "groupRemoved":
      if (state.community === ev.id) setState("community", null);
      return setState("groups", produce((g) => void delete g[ev.id]));
    case "typing": {
      const key = `${ev.chat}|${ev.user}`;
      clearTimeout(typingTimers.get(key));
      if (ev.state === "idle") {
        setState("typing", ev.chat, produce((t) => t && void delete t[ev.user]));
      } else {
        if (!state.typing[ev.chat]) setState("typing", ev.chat, {});
        setState("typing", ev.chat, ev.user, ev.state);
        typingTimers.set(key, setTimeout(() => setState("typing", ev.chat, produce((t) => t && void delete t[ev.user])), 25_000));
      }
      return;
    }
    case "presence":
      return setState("presence", ev.jid, { online: ev.online, lastSeen: ev.lastSeen });
    case "settings":
      return setState("settings", reconcile({ ...DEFAULT_SETTINGS, ...ev.settings }));
    case "privacy":
      return setState("privacy", reconcile(ev.privacy));
    case "call":
      return setState("call", ev.ringing ? { id: ev.id, from: ev.from, video: ev.video } : null);
    case "recording":
      if (state.recording) {
        setState("recording", produce((r) => {
          if (!r) return;
          r.elapsed = ev.elapsed;
          r.levels.push(ev.level);
          if (r.levels.length > 120) r.levels.shift();
        }));
      }
      return;
    case "toast":
      return toast(ev.text, ev.tone === "error" ? "error" : "info");
    case "badge":
      return setState("badge", ev.count);
    case "stickers":
      return setStickerRev((n) => n + 1);
    case "notify":
    case "progress":
      return;
  }
}

function onMessage(m: Message, fresh: boolean) {
  const c = state.convo;
  if (c.chat !== m.chat) return;
  const idx = c.messages.findIndex((x) => x.id === m.id);
  if (idx >= 0) {
    setState("convo", "messages", idx, reconcile(m));
  } else if (!c.detached) {
    const last = c.messages[c.messages.length - 1];
    if (!last || m.ts > last.ts || (m.ts === last.ts && m.seq > last.seq)) {
      if (fresh) {
        freshIds.add(m.id);
        setTimeout(() => freshIds.delete(m.id), 900);
      }
      setState("convo", "messages", c.messages.length, m);
      if (m.fromMe && fresh) setStick((n) => n + 1);
    }
  }
  const pinIdx = c.pinned.findIndex((x) => x.id === m.id);
  const pinnedNow = m.pinnedUntil > Date.now() && m.content.type !== "deleted";
  if (pinIdx >= 0 && !pinnedNow) setState("convo", "pinned", (l) => l.filter((x) => x.id !== m.id));
  else if (pinIdx >= 0) setState("convo", "pinned", pinIdx, reconcile(m));
  else if (pinnedNow) setState("convo", "pinned", (l) => [m, ...l]);
  if (state.reply?.id === m.id && m.content.type === "deleted") setState("reply", null);
}

// ───────────────────────────── conversation ─────────────────────────────

const cursorOf = (m: Message): Cursor => ({ ts: m.ts, seq: m.seq });
let openToken = 0;

interface Opened {
  messages: Message[];
  pinned: Message[];
  unread: number;
  group: Group | null;
}

export async function openChat(id: Jid, jumpTo?: string) {
  if (state.convo.chat === id && !jumpTo) return;
  const token = ++openToken;
  saveDraftNow();
  const res = await act<Opened>("openChat", { chat: id });
  if (!res || token !== openToken) return;
  const incoming = res.messages.filter((m) => !m.fromMe && m.content.type !== "system");
  const firstUnread = res.unread > 0 ? incoming[incoming.length - res.unread] : undefined;
  batch(() => {
    setState("convo", reconcile({
      ...emptyConvo(),
      chat: id,
      messages: res.messages,
      pinned: res.pinned,
      atStart: res.messages.length < 20,
      unreadFrom: firstUnread?.id ?? null,
      unreadCount: firstUnread ? res.unread : 0,
    }, { key: "id" }));
    setState({ reply: null, editing: null, selecting: false, lightbox: null });
    setState("selected", reconcile({}));
    if (res.group) setState("groups", res.group.id, reconcile(res.group));
    if (state.panel === "search") setState("panel", null);
  });
  if (jumpTo) void jumpToMessage(jumpTo);
}

export function closeChat() {
  saveDraftNow();
  openToken++;
  setState("convo", reconcile(emptyConvo()));
  setState({ reply: null, editing: null, selecting: false, panel: null });
  void call("closeChat").catch(() => {});
}

async function reloadHistory(chat: Jid) {
  const first = state.convo.messages[0];
  const older = await call<Message[]>("messages", { chat, before: first ? cursorOf(first) : null, limit: 60 }).catch(() => []);
  if (state.convo.chat !== chat) return;
  if (older.length) setState("convo", "messages", (l) => [...older.filter((o) => !l.some((x) => x.id === o.id)), ...l]);
  else if (!state.convo.messages.length) {
    const page = await call<Message[]>("messages", { chat, limit: 60 }).catch(() => []);
    if (state.convo.chat === chat) setState("convo", "messages", reconcile(page, { key: "id" }));
  }
}

export async function loadOlder() {
  const c = state.convo;
  if (!c.chat || c.loading || c.atStart || !c.messages.length) return;
  setState("convo", "loading", true);
  const chat = c.chat;
  const older = await call<Message[]>("messages", { chat, before: cursorOf(c.messages[0]), limit: 60 }).catch(() => []);
  if (state.convo.chat !== chat) return;
  batch(() => {
    if (older.length) setState("convo", "messages", (l) => [...older, ...l]);
    if (older.length < 60) setState("convo", "atStart", true);
    setState("convo", "loading", false);
  });
}

export async function loadNewer() {
  const c = state.convo;
  if (!c.chat || c.loading || !c.detached || !c.messages.length) return;
  setState("convo", "loading", true);
  const chat = c.chat;
  const newer = await call<Message[]>("messages", { chat, after: cursorOf(c.messages[c.messages.length - 1]), limit: 60 }).catch(() => []);
  if (state.convo.chat !== chat) return;
  batch(() => {
    if (newer.length) setState("convo", "messages", (l) => [...l, ...newer]);
    if (newer.length < 60) setState("convo", "detached", false);
    setState("convo", "loading", false);
  });
}

export async function jumpToLatest() {
  const chat = state.convo.chat;
  if (!chat) return;
  if (state.convo.detached) {
    const page = await call<Message[]>("messages", { chat, limit: 60 }).catch(() => []);
    if (state.convo.chat !== chat) return;
    batch(() => {
      setState("convo", "messages", reconcile(page, { key: "id" }));
      setState("convo", { detached: false, atStart: page.length < 60 });
    });
  }
  setStick((n) => n + 1);
}

let jumpN = 0;
export async function jumpToMessage(id: string) {
  const chat = state.convo.chat;
  if (!chat) return;
  if (!state.convo.messages.some((m) => m.id === id)) {
    const page = await call<Message[]>("messagesAround", { chat, id }).catch(() => []);
    if (state.convo.chat !== chat) return;
    if (!page.some((m) => m.id === id)) return toast("That message is no longer available");
    batch(() => {
      setState("convo", "messages", reconcile(page, { key: "id" }));
      setState("convo", { detached: true, atStart: false });
    });
  }
  setState("convo", "flash", id);
  setJump({ id, n: ++jumpN });
  setTimeout(() => state.convo.flash === id && setState("convo", "flash", null), 1800);
}

/** Opens the chat a message lives in and scrolls to it. */
export async function revealMessage(m: { chat: Jid; id: string }) {
  if (state.convo.chat === m.chat) return jumpToMessage(m.id);
  await openChat(m.chat, m.id);
}

// ───────────────────────────── composer ─────────────────────────────

let draftChat: Jid | null = null;
let draftText = "";
let draftTimer: ReturnType<typeof setTimeout> | undefined;

export function noteDraft(chat: Jid, text: string) {
  draftChat = chat;
  draftText = text;
  clearTimeout(draftTimer);
  draftTimer = setTimeout(saveDraftNow, 700);
}

export function saveDraftNow() {
  clearTimeout(draftTimer);
  if (draftChat !== null && state.chats[draftChat] && (state.chats[draftChat].draft ?? "") !== draftText.trim()) {
    void call("draft", { chat: draftChat, text: draftText }).catch(() => {});
  }
  draftChat = null;
}

export async function send(content: Outgoing, opts: { chat?: Jid; mentions?: Jid[]; keepReply?: boolean } = {}) {
  const chat = opts.chat ?? state.convo.chat;
  if (!chat) return;
  const replyTo = !opts.chat && state.reply ? state.reply.id : null;
  if (!opts.keepReply) setState("reply", null);
  if (state.convo.detached && chat === state.convo.chat) await jumpToLatest();
  return act<Message>("send", { chat, content, replyTo, mentions: opts.mentions ?? [] });
}

// ───────────────────────────── selection ─────────────────────────────

export function toggleSelect(id: string) {
  setState("selecting", true);
  if (state.selected[id]) setState("selected", produce((s) => void delete s[id]));
  else setState("selected", id, true);
}

export function clearSelection() {
  setState("selecting", false);
  setState("selected", reconcile({}));
}

export function selectedMessages(): Message[] {
  return state.convo.messages.filter((m) => state.selected[m.id]);
}

// ───────────────────────────── misc actions ─────────────────────────────

export function openModal(type: string, props?: Record<string, unknown>) {
  setState("modal", { type, props });
}

export function closeModal() {
  setState("modal", null);
}

export function updateSettings(patch: Partial<Settings>) {
  const next = { ...state.settings, ...patch };
  setState("settings", reconcile(next));
  void act("setSettings", { settings: next });
}

export async function startChat(jid: Jid) {
  await act("startChat", { jid });
  setState({ view: "chats", query: "" });
  await openChat(jid);
}

export function typingLabel(chatId: Jid): string | null {
  const t = state.typing[chatId];
  if (!t) return null;
  const users = Object.keys(t);
  if (!users.length) return null;
  const verb = (u: string) => (t[u] === "recording" ? "recording audio" : "typing");
  if (!isGroupJid(chatId)) return `${verb(users[0])}…`;
  if (users.length === 1) return `${contactName(users[0], { short: true }).replace(/^~/, "")} is ${verb(users[0])}…`;
  return `${users.length} people are typing…`;
}
