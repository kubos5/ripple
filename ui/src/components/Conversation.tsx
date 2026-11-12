// The open chat: header, pinned bar, virtualised message list.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { Virtualizer, type VirtualizerHandle } from "virtua/solid";
import { daysApart, dayLabel, lastSeen, plural, tone } from "../lib/format";
import {
  act, chatName, clearSelection, contactName, derived, isMuted, jumpSignal, jumpToLatest, jumpToMessage, loadNewer,
  loadOlder, openModal, previewText, selectedMessages, setState, state, stickSignal, toast, typingLabel,
} from "../lib/state";
import type { Chat, Message } from "../lib/types";
import { MessageRow, type RowFlags, stopAudio } from "./Bubble";
import { Composer } from "./Composer";
import { KIND_ICON, bumpStars, chatMenu } from "./Sidebar";
import { Avatar, Icon, IconButton, openMenu } from "./ui";

// ───────────────────────────── header ─────────────────────────────

function Subtitle(props: { chat: Chat }) {
  const c = () => props.chat;
  const text = createMemo<{ t: string; live?: boolean }>(() => {
    const typing = typingLabel(c().id);
    if (typing) return { t: typing, live: true };
    if (c().id === state.me?.id) return { t: "Message yourself" };
    if (c().kind === "direct") {
      const p = state.presence[c().id];
      if (p?.online) return { t: "online", live: true };
      if (p?.lastSeen) return { t: lastSeen(p.lastSeen, state.now) };
      const contact = state.contacts[c().id];
      return { t: contact?.isBusiness ? "Business account" : "Click for contact info" };
    }
    const g = state.groups[c().id];
    if (!g) return { t: "Group" };
    if (!g.joined) return { t: "You're no longer a member" };
    if (c().kind === "announcement") return { t: `Announcements · ${plural(g.size || g.participants.length, "member")}` };
    const list = g.participants.map((p) => contactName(p.id, { short: true }).replace(/^~/, ""));
    return { t: list.length > 6 ? `${list.slice(0, 6).join(", ")} and ${list.length - 6} more` : list.join(", ") };
  });
  return (
    <div class="sub truncate">
      <Show when={text().t} keyed>
        {(t) => <span class={`truncate ${text().live ? "live" : ""}`}>{t}</span>}
      </Show>
    </div>
  );
}

function Header(props: { chat: Chat }) {
  const c = () => props.chat;
  const more = (e: MouseEvent) =>
    openMenu(e.currentTarget as HTMLElement, [
      { label: c().kind === "direct" ? "Contact info" : "Group info", icon: "info", run: () => setState("panel", "info") },
      { label: "Select messages", icon: "check-circle", run: () => setState("selecting", true) },
      { label: "Starred messages", icon: "star", run: () => setState("panel", "starred") },
      { label: "Disappearing messages", icon: "timer", hidden: c().readOnly, run: () => openModal("ephemeral", { chat: c().id }) },
      { sep: true },
      ...chatMenu(c()),
    ], { align: "right" });
  return (
    <header class="convo-head drag">
      <button class="who no-drag" onClick={() => setState("panel", state.panel === "info" ? null : "info")}>
        <Avatar id={c().id} size={38} online={c().kind === "direct" && state.presence[c().id]?.online} />
        <div class="grow">
          <div class="title">
            <span class="truncate">{chatName(c().id)}</span>
            <Show when={state.contacts[c().id]?.verifiedName}><Icon name="check-circle-fill" /></Show>
            <Show when={isMuted(c())}><Icon name="bell-slash-fill" style={{ color: "var(--text-3)" }} /></Show>
            <Show when={c().ephemeral > 0}><Icon name="timer" style={{ color: "var(--text-3)" }} /></Show>
          </div>
          <Subtitle chat={c()} />
        </div>
      </button>
      <IconButton class="no-drag" icon="magnifying-glass" tip="Search in chat" on={state.panel === "search"} onClick={() => setState("panel", state.panel === "search" ? null : "search")} />
      <IconButton class="no-drag" icon="dots-three-vertical" tip="More" onClick={more} />
    </header>
  );
}

function SelectBar() {
  const picked = () => selectedMessages();
  const n = () => picked().length;
  const ids = () => picked().map((m) => m.id);
  const chat = () => state.convo.chat!;
  return (
    <div class="select-bar">
      <IconButton icon="x" tip="Cancel" onClick={clearSelection} />
      <span class="count">{n() ? plural(n(), "message") + " selected" : "Select messages"}</span>
      <IconButton icon="star" tip="Star" disabled={!n()} onClick={async () => { await act("star", { chat: chat(), ids: ids(), on: !picked().every((m) => m.starred) }); bumpStars((x) => x + 1); clearSelection(); }} />
      <IconButton icon="copy" tip="Copy" disabled={!n()} onClick={() => {
        const text = picked().map((m) => (m.content.type === "text" ? m.content.text : "caption" in m.content ? m.content.caption : "")).filter(Boolean).join("\n");
        void navigator.clipboard.writeText(text);
        toast("Copied", "success");
        clearSelection();
      }} />
      <IconButton icon="share-fat" tip="Forward" disabled={!n()} onClick={() => openModal("forward", { chat: chat(), ids: ids() })} />
      <IconButton icon="trash" tip="Delete" disabled={!n()} onClick={() => openModal("delete", { chat: chat(), ids: ids() })} />
    </div>
  );
}

function PinBar() {
  const pins = () => state.convo.pinned;
  const [idx, setIdx] = createSignal(0);
  createEffect(() => { if (idx() >= pins().length) setIdx(0); });
  const cur = () => pins()[Math.min(idx(), pins().length - 1)];
  const label = (m: Message) => {
    const c = m.content;
    if (c.type === "text") return previewText(c.text);
    if ("caption" in c && c.caption) return previewText(c.caption);
    if (c.type === "poll") return c.question;
    if (c.type === "event") return c.name;
    if (c.type === "document") return c.name;
    return { image: "Photo", video: "Video", audio: "Voice message", sticker: "Sticker", location: "Location", contacts: "Contact" }[c.type as string] ?? "Message";
  };
  return (
    <Show when={cur()}>
      {(m) => (
        <button
          class="pinbar"
          onClick={() => { void jumpToMessage(m().id); setIdx((i) => (i + 1) % pins().length); }}
          onContextMenu={(e) => openMenu(e, [{ label: "Unpin", icon: "push-pin", run: () => void act("pinMessage", { chat: m().chat, id: m().id, secs: 0 }) }])}
        >
          <Show when={pins().length > 1}>
            <span class="steps"><For each={pins()}>{(_, i) => <i class={i() === idx() ? "on" : ""} />}</For></span>
          </Show>
          <div class="grow">
            <div class="label">Pinned message{pins().length > 1 ? ` ${idx() + 1} of ${pins().length}` : ""}</div>
            <div class="text truncate">
              <Show when={KIND_ICON[m().content.type]}>{(i) => <Icon name={i()} size={14} style={{ display: "inline", "vertical-align": "-2px", "margin-right": "4px" }} />}</Show>
              {label(m())}
            </div>
          </div>
          <Icon name="push-pin-fill" />
        </button>
      )}
    </Show>
  );
}

// ───────────────────────────── list ─────────────────────────────

interface Item {
  k: "top" | "day" | "unread" | "msg" | "album";
  key: string;
  ts: number;
  m?: Message;
}

const GAP = 8 * 60_000;
const breaksGroup = (m: Message) => m.content.type === "system" || m.content.type === "call";
const isVisual = (m: Message) => !m.viewOnce && (m.content.type === "image" || (m.content.type === "video" && !m.content.gif));

function MessageList(props: { chat: Chat }) {
  let scroller!: HTMLDivElement;
  let handle: VirtualizerHandle | undefined;
  const cache = new Map<string, Item>();
  const [shift, setShift] = createSignal(false);
  const [atBottom, setAtBottom] = createSignal(true);
  const [below, setBelow] = createSignal(0);
  let pinned = true; // non-reactive mirror of atBottom for scroll handlers

  const layout = createMemo(() => {
    const msgs = state.convo.messages;
    const items: Item[] = [];
    const flags = new Map<string, RowFlags>();
    const albums = new Map<string, Message[]>();
    const item = (k: Item["k"], key: string, ts: number, m?: Message) => {
      let it = cache.get(key);
      if (!it || it.m !== m) {
        it = { k, key, ts, m };
        cache.set(key, it);
      }
      items.push(it);
      return it;
    };
    item("top", "top", 0);
    let prev: Message | undefined;
    let forceFirst = true;
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      if (!prev || daysApart(prev.ts, m.ts) !== 0) {
        const d = new Date(m.ts);
        item("day", `day-${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`, m.ts);
        forceFirst = true;
      }
      if (m.id === state.convo.unreadFrom) {
        item("unread", "unread", m.ts);
        forceFirst = true;
      }
      // Gather an album: consecutive photos/videos from one sender sharing an album id.
      let group: Message[] | undefined;
      if (m.album && isVisual(m)) {
        let j = i;
        while (j + 1 < msgs.length && msgs[j + 1].album === m.album && msgs[j + 1].sender === m.sender && isVisual(msgs[j + 1])) j++;
        if (j > i) {
          group = msgs.slice(i, j + 1);
          i = j;
        }
      }
      const cur = group ? group[group.length - 1] : m;
      const next = msgs[i + 1];
      const first = forceFirst || !prev || prev.sender !== m.sender || breaksGroup(prev) || m.ts - prev.ts > GAP;
      const last = !next || next.sender !== cur.sender || breaksGroup(next) || next.ts - cur.ts > GAP || daysApart(cur.ts, next.ts) !== 0 || next.id === state.convo.unreadFrom;
      flags.set(m.id, { first, last });
      if (group) {
        albums.set(m.id, group);
        item("album", `m-${m.id}`, m.ts, m);
      } else {
        item("msg", `m-${m.id}`, m.ts, m);
      }
      forceFirst = false;
      prev = cur;
    }
    if (cache.size > items.length + 400) {
      const live = new Set(items.map((x) => x.key));
      for (const k of cache.keys()) if (!live.has(k)) cache.delete(k);
    }
    return { items, flags, albums };
  });

  // Instant on purpose: the new bubble's own entrance supplies the motion, and
  // smooth-scrolling a virtualised list can briefly show unrendered rows.
  const toBottom = () => {
    const n = layout().items.length;
    if (!handle || !n) return;
    handle.scrollToIndex(n - 1, { align: "end" });
    scroller.scrollTop = scroller.scrollHeight;
  };

  onMount(() => {
    // First paint: land on the unread divider if there is one, otherwise the newest message.
    const start = () => {
      const idx = layout().items.findIndex((x) => x.k === "unread");
      if (idx >= 0 && handle) {
        handle.scrollToIndex(idx, { align: "start", offset: -70 });
        pinned = false;
        setAtBottom(false);
      } else toBottom();
    };
    start();
    requestAnimationFrame(start);
    // Keep the view glued to the bottom while late content (images) changes height.
    const ro = new ResizeObserver(() => { if (pinned) scroller.scrollTop = scroller.scrollHeight; });
    if (scroller.firstElementChild) ro.observe(scroller.firstElementChild);
    onCleanup(() => ro.disconnect());
  });

  // New messages: follow if we were at the bottom, otherwise count them on the jump button.
  let lastLen = state.convo.messages.length;
  let lastId = state.convo.messages[lastLen - 1]?.id;
  createEffect(on(() => state.convo.messages.length, (len) => {
    const tail = state.convo.messages[len - 1];
    const appended = len > lastLen && tail && tail.id !== lastId;
    if (appended) {
      if (pinned) queueMicrotask(toBottom);
      else if (!tail.fromMe) setBelow((n) => n + 1);
    }
    lastLen = len;
    lastId = tail?.id;
  }, { defer: true }));

  createEffect(on(stickSignal, () => {
    pinned = true;
    setAtBottom(true);
    setBelow(0);
    queueMicrotask(toBottom);
    requestAnimationFrame(toBottom);
  }, { defer: true }));

  createEffect(on(jumpSignal, (j) => {
    if (!j) return;
    const go = () => {
      const { items, albums } = layout();
      let idx = items.findIndex((x) => x.m?.id === j.id);
      if (idx < 0) idx = items.findIndex((x) => x.k === "album" && albums.get(x.m!.id)?.some((a) => a.id === j.id));
      if (idx >= 0) handle?.scrollToIndex(idx, { align: "center" });
    };
    pinned = false;
    setAtBottom(false);
    queueMicrotask(go);
    requestAnimationFrame(go);
  }, { defer: true }));

  createEffect(on(() => typingLabel(props.chat.id), () => { if (pinned) requestAnimationFrame(() => { scroller.scrollTop = scroller.scrollHeight; }); }, { defer: true }));

  let loadingOlder = false;
  const onScroll = () => {
    const gap = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    const bottom = gap < 60;
    if (bottom !== pinned) {
      pinned = bottom && !state.convo.detached;
      setAtBottom(pinned);
    }
    if (bottom && below()) setBelow(0);
    if (scroller.scrollTop < 700 && !loadingOlder && !state.convo.atStart && state.convo.messages.length) {
      loadingOlder = true;
      setShift(true);
      void loadOlder().finally(() => {
        requestAnimationFrame(() => {
          setShift(false);
          loadingOlder = false;
        });
      });
    }
    if (bottom && state.convo.detached) void loadNewer();
  };

  const typers = () => Object.entries(state.typing[props.chat.id] ?? {});

  return (
    <>
      <div class="msg-scroll" ref={scroller} onScroll={onScroll}>
        <Virtualizer ref={(h) => (handle = h)} data={layout().items} shift={shift()} bufferSize={500}>
          {(it) => (
            <Switch>
              <Match when={it.k === "top"}>
                <Show when={state.convo.atStart} fallback={<div class="list-note"><Show when={state.convo.loading}><div class="spinner" /></Show></div>}>
                  <div class="list-note">
                    <div class="card">
                      <Icon name="lock-fill" />
                      {props.chat.id === state.me?.id
                        ? "Notes you send here are only visible to you, on all your linked devices."
                        : "Messages and calls are end-to-end encrypted. Only people in this chat can read or listen to them."}
                    </div>
                  </div>
                </Show>
              </Match>
              <Match when={it.k === "day"}>
                <div class="day"><span>{dayLabel(it.ts, state.now)}</span></div>
              </Match>
              <Match when={it.k === "unread"}>
                <div class="unread-line">{plural(state.convo.unreadCount, "unread message")}</div>
              </Match>
              <Match when={it.m}>
                {(m) => (
                  <MessageRow
                    m={m()}
                    flags={() => layout().flags.get(m().id) ?? { first: true, last: true }}
                    album={it.k === "album" ? () => layout().albums.get(m().id) : undefined}
                  />
                )}
              </Match>
            </Switch>
          )}
        </Virtualizer>
        <Show when={typers().length}>
          <div class="typing-row">
            <Show when={props.chat.kind !== "direct"}><Avatar id={typers()[0][0]} size={30} /></Show>
            <div class="typing-bubble" style={{ "--tone": `var(--tone-${tone(typers()[0][0])})` }}>
              <Show when={typers()[0][1] === "recording"} fallback={<><i /><i /><i /></>}>
                <Icon name="microphone-fill" /><i /><i /><i />
              </Show>
            </div>
          </div>
        </Show>
        <div class="msg-pad" />
      </div>
      <Show when={!atBottom() || state.convo.detached}>
        <button class="jump" aria-label="Jump to latest" onClick={() => void jumpToLatest()}>
          <Icon name="arrow-down" />
          <Show when={below()}><span class="badge">{below()}</span></Show>
        </button>
      </Show>
    </>
  );
}

// ───────────────────────────── container ─────────────────────────────

export function Conversation() {
  const [dragDepth, setDragDepth] = createSignal(0);
  const hasFiles = (e: DragEvent) => [...(e.dataTransfer?.types ?? [])].includes("Files");
  createEffect(on(() => state.convo.chat, () => stopAudio(), { defer: true }));
  return (
    <Show when={derived.activeChat()} keyed fallback={<Blank />}>
      {(_chat) => {
        const chat = () => derived.activeChat()!;
        return (
          <Show when={state.convo.chat} keyed>
            {(_id) => (
              <section
                class="convo swap"
                onDragEnter={(e) => { if (hasFiles(e)) { e.preventDefault(); setDragDepth((d) => d + 1); } }}
                onDragOver={(e) => hasFiles(e) && e.preventDefault()}
                onDragLeave={(e) => hasFiles(e) && setDragDepth((d) => Math.max(0, d - 1))}
                onDrop={(e) => {
                  if (!hasFiles(e)) return;
                  e.preventDefault();
                  setDragDepth(0);
                  const files = [...(e.dataTransfer?.files ?? [])];
                  if (files.length && !chat().readOnly) openModal("attach", { files });
                }}
              >
                <Show when={state.selecting} fallback={<Header chat={chat()} />}><SelectBar /></Show>
                <PinBar />
                <MessageList chat={chat()} />
                <Composer chat={chat()} />
                <Show when={dragDepth() > 0 && !chat().readOnly}>
                  <div class="dropzone"><div><Icon name="images" />Drop to send to {chatName(chat().id)}</div></div>
                </Show>
              </section>
            )}
          </Show>
        );
      }}
    </Show>
  );
}

function Blank() {
  const mod = () => (state.platform === "macos" ? "⌘" : "Ctrl");
  return (
    <section class="blank drag">
      <div class="ripple-mark"><i /><i /><i /><b /></div>
      <h2 class="display">Ripple</h2>
      <p>Pick a conversation from the list, or start a new one. Your messages stay end-to-end encrypted.</p>
      <div class="keys">
        <span><span class="kbd">{mod()}</span><span class="kbd">K</span> Jump to anything</span>
        <span><span class="kbd">{mod()}</span><span class="kbd">N</span> New chat</span>
        <span><span class="kbd">{mod()}</span><span class="kbd">,</span> Settings</span>
      </div>
    </section>
  );
}
