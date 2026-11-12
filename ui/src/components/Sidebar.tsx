// Left side of the app: navigation rail and the list pane next to it.

import { For, type JSX, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onCleanup } from "solid-js";
import { VList } from "virtua/solid";
import { call } from "../lib/ipc";
import { clamp, debounce, listTime, plural } from "../lib/format";
import {
  act, chatName, contactName, derived, isAdmin, isMuted, openChat, openModal, previewText, revealMessage, setState,
  startChat, state, typingLabel, updateSettings, type Filter, type View,
} from "../lib/state";
import type { Chat, Group, Message, Status } from "../lib/types";
import { Avatar, Icon, IconButton, type IconName, type MenuEntry, openMenu } from "./ui";

// ───────────────────────────── rail ─────────────────────────────

export function Logo(props: { class?: string }) {
  return (
    <svg class={props.class} viewBox="0 0 36 36" fill="none" aria-hidden="true">
      <circle cx="18" cy="18" r="4" fill="currentColor" />
      <circle cx="18" cy="18" r="9.5" stroke="currentColor" stroke-width="2.2" opacity="0.62" />
      <circle cx="18" cy="18" r="15.5" stroke="currentColor" stroke-width="2.2" opacity="0.26" />
    </svg>
  );
}

export function Rail() {
  const go = (v: View) => {
    setState({ view: v, query: "" });
    if (v !== "communities") setState("community", null);
  };
  const item = (v: View, icon: IconName, label: string, count?: () => number) => (
    <button
      class={`rail-btn ${state.view === v ? "on" : ""}`}
      data-tip={label}
      data-tip-pos="right"
      aria-label={label}
      onClick={() => go(v)}
    >
      <span class="rail-face">
        <Icon name={state.view === v ? (`${icon}-fill` as IconName) : icon} />
        <Show when={count?.()}>{(n) => <span class="badge">{n() > 99 ? "99+" : n()}</span>}</Show>
      </span>
    </button>
  );
  return (
    <nav class="rail drag">
      <Logo class="rail-mark" />
      <div class="col no-drag" style={{ gap: "4px", "align-items": "center" }}>
        {item("chats", "chats-circle", "Chats", derived.unreadChats)}
        {item("communities", "users-three", "Communities", derived.communityUnread)}
        {item("starred", "star", "Starred messages")}
        {item("archived", "archive", "Archived", derived.archivedUnread)}
      </div>
      <div class="rail-spacer" />
      <div class="col no-drag" style={{ gap: "4px", "align-items": "center" }}>
        <button class="rail-btn" data-tip="Settings" data-tip-pos="right" aria-label="Settings" onClick={() => openModal("settings")}>
          <span class="rail-face"><Icon name="gear-six" /></span>
        </button>
        <Show when={state.me}>
          {(me) => (
            <button class="rail-me" data-tip="Profile" data-tip-pos="right" aria-label="Profile" onClick={() => openModal("settings", { tab: "profile" })}>
              <Avatar id={me().id} size={34} name={me().pushName ?? "Me"} />
            </button>
          )}
        </Show>
      </div>
    </nav>
  );
}

// ───────────────────────────── chat rows ─────────────────────────────

export const KIND_ICON: Record<string, IconName> = {
  image: "camera", video: "video-camera", gif: "gif", voice: "microphone", audio: "headphones", document: "file-text",
  sticker: "sticker", location: "map-pin", contacts: "user", poll: "chart-bar", event: "calendar-blank", call: "phone-x",
  deleted: "prohibit",
};

export function Ticks(props: { status: Status; class?: string }) {
  const icon = (): IconName =>
    props.status === "pending" ? "clock" : props.status === "failed" ? "warning-circle-fill" : props.status === "sent" ? "check" : "checks";
  return (
    <Icon
      name={icon()}
      class={`tick ${props.status === "read" || props.status === "played" ? "read" : ""} ${props.class ?? ""}`}
      style={props.status === "failed" ? { color: "var(--danger)" } : undefined}
    />
  );
}

export function chatMenu(chat: Chat): MenuEntry[] {
  const group = chat.kind !== "direct";
  const joined = !group || state.groups[chat.id]?.joined !== false;
  const isSelf = chat.id === state.me?.id;
  return [
    {
      label: chat.unread || chat.markedUnread ? "Mark as read" : "Mark as unread",
      icon: "envelope-simple",
      run: () => void act(chat.unread || chat.markedUnread ? "markRead" : "markUnread", { chat: chat.id }),
    },
    { label: chat.pinned ? "Unpin" : "Pin to top", icon: "push-pin", hidden: chat.archived, run: () => void act("pinChat", { chat: chat.id, on: !chat.pinned }) },
    {
      label: isMuted(chat) ? "Unmute" : "Mute notifications",
      icon: isMuted(chat) ? "bell" : "bell-slash",
      hidden: isSelf,
      run: () => (isMuted(chat) ? void act("mute", { chat: chat.id, until: 0 }) : openModal("mute", { chat: chat.id })),
    },
    { label: chat.favorite ? "Remove from favourites" : "Add to favourites", icon: "heart", run: () => void act("favorite", { chat: chat.id, on: !chat.favorite }) },
    { label: chat.archived ? "Unarchive" : "Archive", icon: "archive", run: () => void act("archive", { chat: chat.id, on: !chat.archived }) },
    { sep: true },
    { label: "Clear messages", icon: "broom", run: () => openModal("confirmClear", { chat: chat.id }) },
    { label: "Exit group", icon: "sign-out", danger: true, hidden: !group || !joined, run: () => openModal("confirmLeave", { chat: chat.id }) },
    { label: "Delete chat", icon: "trash", danger: true, hidden: group && joined, run: () => openModal("confirmDelete", { chat: chat.id }) },
  ];
}

function PreviewLine(props: { chat: Chat }) {
  const typing = () => typingLabel(props.chat.id);
  const last = () => props.chat.last;
  const who = () => {
    const l = last();
    if (!l || props.chat.kind === "direct" || l.kind === "system") return "";
    if (l.fromMe) return "You:";
    return l.sender ? `${contactName(l.sender, { short: true }).replace(/^~/, "")}:` : "";
  };
  return (
    <div class="preview">
      <Switch>
        <Match when={typing()}>
          <span class="typing truncate">{typing()}</span>
        </Match>
        <Match when={props.chat.draft && state.convo.chat !== props.chat.id}>
          <span class="draft">Draft:</span>
          <span class="truncate">{props.chat.draft}</span>
        </Match>
        <Match when={last()}>
          {(l) => (
            <>
              <Show when={l().fromMe && l().kind !== "system" && l().kind !== "deleted"}>
                <Ticks status={l().status} />
              </Show>
              <Show when={who()}><span class="who truncate">{who()}</span></Show>
              <Show when={KIND_ICON[l().kind]}>{(i) => <Icon name={i()} />}</Show>
              <span class="truncate">{previewText(l().text)}</span>
            </>
          )}
        </Match>
        <Match when={true}>
          <span class="truncate faint">{props.chat.kind === "direct" ? "No messages yet" : "You joined this group"}</span>
        </Match>
      </Switch>
    </div>
  );
}

export function ChatRow(props: { chat: Chat }) {
  const c = () => props.chat;
  const unread = () => c().unread > 0 || c().markedUnread;
  return (
    <button
      class={`chat-row ${state.convo.chat === c().id ? "on" : ""} ${unread() ? "unread" : ""}`}
      onClick={() => void openChat(c().id)}
      onContextMenu={(e) => openMenu(e, chatMenu(c()))}
    >
      <Avatar id={c().id} size={48} online={c().kind === "direct" && state.presence[c().id]?.online} />
      <div class="lines">
        <div class="top">
          <span class="name truncate">{chatName(c().id)}</span>
          <span class="time">{listTime(c().last?.ts ?? 0, state.now)}</span>
        </div>
        <div class="bottom">
          <PreviewLine chat={c()} />
          <div class="marks">
            <Show when={c().ephemeral > 0}><Icon name="timer" /></Show>
            <Show when={isMuted(c())}><Icon name="bell-slash-fill" /></Show>
            <Show when={c().pinned > 0 && !unread()}><Icon name="push-pin-fill" /></Show>
            <Show when={c().unreadMentions > 0}><span class="badge at"><Icon name="at" /></span></Show>
            <Show when={unread()}>
              <Show when={c().unread > 0} fallback={<span class={`badge ${isMuted(c()) ? "quiet" : ""}`} style={{ "min-width": "12px", width: "12px", height: "12px", padding: 0 }} />} keyed>
                <span class={`badge ${isMuted(c()) ? "quiet" : ""}`}>{c().unread > 999 ? "999+" : c().unread}</span>
              </Show>
            </Show>
          </div>
        </div>
      </div>
    </button>
  );
}

// ───────────────────────────── search results ─────────────────────────────

function Highlight(props: { text: string; query: string }) {
  const parts = createMemo(() => {
    const words = props.query.trim().split(/\s+/).filter(Boolean).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    if (!words.length) return [props.text];
    return props.text.split(new RegExp(`(${words.join("|")})`, "gi"));
  });
  return <For each={parts()}>{(p, i) => (i() % 2 ? <mark>{p}</mark> : <>{p}</>)}</For>;
}

/** Trims a long message around the first match so the hit is visible in one line. */
function snippet(text: string, query: string) {
  const flat = previewText(text);
  const first = query.trim().split(/\s+/)[0]?.toLowerCase() ?? "";
  const at = flat.toLowerCase().indexOf(first);
  return at > 40 ? `…${flat.slice(at - 24)}` : flat;
}

export function MessageHit(props: { m: Message; query?: string; showChat?: boolean; onPick?: () => void }) {
  const text = () => {
    const c = props.m.content;
    if (c.type === "text") return c.text;
    if ("caption" in c && c.caption) return c.caption;
    if (c.type === "poll") return c.question;
    if (c.type === "event") return c.name;
    if (c.type === "document") return c.name;
    return { image: "Photo", video: "Video", audio: "Audio", sticker: "Sticker", location: "Location", contacts: "Contact" }[c.type as string] ?? "Message";
  };
  return (
    <button class="list-row" onClick={() => (props.onPick ? props.onPick() : void revealMessage(props.m))}>
      <Show when={props.showChat !== false}><Avatar id={props.m.chat} size={40} /></Show>
      <div class="grow">
        <div class="row" style={{ gap: "8px" }}>
          <span class="title truncate grow">
            {props.showChat !== false ? chatName(props.m.chat) : contactName(props.m.sender)}
          </span>
          <span class="faint tnum" style={{ "font-size": "var(--fs-12)" }}>{listTime(props.m.ts, state.now)}</span>
        </div>
        <div class="sub truncate">
          <Show when={props.showChat !== false && (props.m.fromMe || props.m.chat.endsWith("@g.us"))}>
            <span>{contactName(props.m.sender, { short: true })}: </span>
          </Show>
          <Show when={KIND_ICON[props.m.content.type]}>{(i) => <Icon name={i()} size={14} style={{ display: "inline", "vertical-align": "-2px", "margin-right": "4px" }} />}</Show>
          <Highlight text={snippet(text(), props.query ?? "")} query={props.query ?? ""} />
        </div>
      </div>
    </button>
  );
}

function SearchResults(props: { query: string }) {
  const q = () => props.query.trim().toLowerCase();
  const chats = createMemo(() =>
    derived.all().filter((c) => chatName(c.id).toLowerCase().includes(q())).sort((a, b) => b.ts - a.ts).slice(0, 12),
  );
  const contacts = createMemo(() =>
    Object.values(state.contacts)
      .filter((c) => !c.isMe && !state.chats[c.id] && c.name && c.name.toLowerCase().includes(q()))
      .slice(0, 8),
  );
  const [term, setTerm] = createSignal(props.query);
  const push = debounce((v: string) => setTerm(v), 160);
  createEffect(() => push(props.query));
  const [hits] = createResource(term, (t) => (t.trim().length >= 2 ? call<Message[]>("search", { query: t }).catch(() => []) : []));
  const none = () => !chats().length && !contacts().length && !hits.loading && !(hits() ?? []).length;
  return (
    <div class="side-list">
      <Show when={chats().length}>
        <div class="side-section">Chats</div>
        <For each={chats()}>{(c) => <ChatRow chat={c} />}</For>
      </Show>
      <Show when={contacts().length}>
        <div class="side-section">Contacts</div>
        <For each={contacts()}>
          {(c) => (
            <button class="list-row" onClick={() => void startChat(c.id)}>
              <Avatar id={c.id} size={40} />
              <div class="grow">
                <div class="title truncate">{contactName(c.id)}</div>
                <div class="sub truncate">{c.about ?? "Start a chat"}</div>
              </div>
            </button>
          )}
        </For>
      </Show>
      <Show when={(hits() ?? []).length}>
        <div class="side-section">Messages</div>
        <For each={hits()}>{(m) => <MessageHit m={m} query={props.query} />}</For>
      </Show>
      <Show when={none()}>
        <div class="empty">
          <Icon name="magnifying-glass" />
          <strong>No results for “{props.query.trim()}”</strong>
          Try a name, a word from a message, or a phone number.
        </div>
      </Show>
    </div>
  );
}

// ───────────────────────────── views ─────────────────────────────

function ChatList(props: { chats: Chat[]; head?: JSX.Element; empty: JSX.Element }) {
  return (
    <Show when={props.chats.length} fallback={<div class="side-list">{props.head}{props.empty}</div>}>
      <VList data={props.chats} class="side-list" style={{ flex: "1 1 0", "min-height": 0 }} bufferSize={320}>
        {(chat, i) => (
          <>
            <Show when={i() === 0}>{props.head}</Show>
            <ChatRow chat={chat} />
          </>
        )}
      </VList>
    </Show>
  );
}

function SearchBox(props: { placeholder: string }) {
  let input: HTMLInputElement | undefined;
  const onFocusReq = () => input?.focus();
  window.addEventListener("ripple:focus-search", onFocusReq);
  onCleanup(() => window.removeEventListener("ripple:focus-search", onFocusReq));
  return (
    <div class="search">
      <Icon name="magnifying-glass" />
      <input
        ref={input}
        placeholder={props.placeholder}
        value={state.query}
        spellcheck={false}
        onInput={(e) => setState("query", e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setState("query", "");
            e.currentTarget.blur();
          }
        }}
      />
      <Show when={state.query} fallback={<span class="kbd-hint"><span class="kbd">{state.platform === "macos" ? "⌘" : "Ctrl"}</span><span class="kbd">K</span></span>}>
        <IconButton icon="x" small class="clear" tip="Clear" onClick={() => { setState("query", ""); input?.focus(); }} />
      </Show>
    </div>
  );
}

function ChatsView() {
  const filters: [Filter, string][] = [["all", "All"], ["unread", "Unread"], ["favorites", "Favourites"], ["groups", "Groups"]];
  const list = createMemo(() => {
    const base = derived.inbox();
    switch (state.filter) {
      case "unread": return base.filter((c) => c.unread > 0 || c.markedUnread);
      case "favorites": return base.filter((c) => c.favorite);
      case "groups": return base.filter((c) => c.kind !== "direct");
      default: return base;
    }
  });
  const newMenu = (e: MouseEvent) =>
    openMenu(e.currentTarget as HTMLElement, [
      { label: "New chat", icon: "chat-circle", hint: state.platform === "macos" ? "⌘N" : "Ctrl+N", run: () => openModal("newChat") },
      { label: "New group", icon: "users", run: () => openModal("newGroup") },
      { label: "New community", icon: "users-three", run: () => openModal("newCommunity") },
      { sep: true },
      { label: "Join group with a link", icon: "link-simple", run: () => openModal("joinGroup") },
    ], { align: "right" });
  return (
    <>
      <div class="side-head drag">
        <h1>Chats</h1>
        <IconButton class="no-drag" icon="note-pencil" tip="New chat" onClick={newMenu} />
      </div>
      <SearchBox placeholder="Search" />
      <Show when={!state.query.trim()} fallback={<SearchResults query={state.query} />}>
        <div class="chips">
          <For each={filters}>
            {([f, label]) => (
              <button class={`chip ${state.filter === f ? "on" : ""}`} onClick={() => setState("filter", f)}>
                {label}
                <Show when={f === "unread" && derived.unreadChats() > 0}><span class="tnum">{derived.unreadChats()}</span></Show>
              </button>
            )}
          </For>
        </div>
        <Show when={state.session.state === "offline" || state.session.state === "connecting"}>
          <div class="banner">
            <Icon name="wifi-slash" />
            {state.session.state === "offline" ? "No connection. Reconnecting…" : "Connecting…"}
          </div>
        </Show>
        <ChatList
          chats={list()}
          head={
            <Show when={state.filter === "all" && derived.archived().length > 0}>
              <button class="archived-link" onClick={() => setState("view", "archived")}>
                <Icon name="archive" />
                Archived
                <span class="count">{derived.archivedUnread() || ""}</span>
              </button>
            </Show>
          }
          empty={
            <div class="empty">
              <Icon name={state.filter === "favorites" ? "heart" : "chats-circle"} />
              <strong>{state.filter === "unread" ? "You're all caught up" : state.filter === "favorites" ? "No favourites yet" : state.filter === "groups" ? "No groups yet" : "No chats yet"}</strong>
              {state.filter === "favorites" ? "Right-click a chat to add it to your favourites." : state.filter === "all" ? "Start a conversation with the pencil button above." : ""}
            </div>
          }
        />
      </Show>
    </>
  );
}

function ArchivedView() {
  return (
    <>
      <div class="side-head sub drag">
        <IconButton class="no-drag" icon="arrow-left" tip="Back" onClick={() => setState("view", "chats")} />
        <h1>Archived</h1>
      </div>
      <ChatList
        chats={derived.archived()}
        empty={<div class="empty"><Icon name="archive" /><strong>Nothing archived</strong>Archived chats stay out of your way but keep their messages.</div>}
      />
    </>
  );
}

export const [starVersion, bumpStars] = createSignal(0);

function StarredView() {
  const [list] = createResource(starVersion, () => call<Message[]>("starred").catch(() => []));
  return (
    <>
      <div class="side-head drag"><h1>Starred</h1></div>
      <div class="side-list">
        <For each={list()} fallback={
          <Show when={!list.loading}>
            <div class="empty"><Icon name="star" /><strong>No starred messages</strong>Star a message to find it here later.</div>
          </Show>
        }>
          {(m) => <MessageHit m={m} />}
        </For>
      </div>
    </>
  );
}

function subgroupsOf(id: string): Group[] {
  return Object.values(state.groups)
    .filter((g) => g.parent === id && !g.isAnnouncement)
    .sort((a, b) => Number(b.joined) - Number(a.joined) || a.subject.localeCompare(b.subject));
}

function CommunityHome(props: { group: Group }) {
  const g = () => props.group;
  const announcement = () => Object.values(state.groups).find((x) => x.parent === g().id && x.isAnnouncement);
  const subs = createMemo(() => subgroupsOf(g().id));
  const admin = () => isAdmin(g().id);
  const more = (e: MouseEvent) =>
    openMenu(e.currentTarget as HTMLElement, [
      { label: "Community info", icon: "info", run: () => openModal("communityInfo", { id: g().id }) },
      { label: "Add existing groups", icon: "plus", hidden: !admin(), run: () => openModal("linkGroups", { id: g().id }) },
      { label: "Create a group here", icon: "users", hidden: !admin(), run: () => openModal("newGroup", { parent: g().id }) },
      { sep: true, hidden: !admin() },
      { label: "Deactivate community", icon: "trash", danger: true, hidden: !admin(), run: () => openModal("confirmDeactivate", { id: g().id }) },
    ], { align: "right" });
  return (
    <>
      <div class="side-head sub drag">
        <IconButton class="no-drag" icon="arrow-left" tip="All communities" onClick={() => setState("community", null)} />
        <h1 class="truncate">{g().subject}</h1>
        <IconButton class="no-drag" icon="dots-three-vertical" tip="More" onClick={more} />
      </div>
      <div class="side-list">
        <button class="community-hero" onClick={() => openModal("communityInfo", { id: g().id })}>
          <Avatar id={g().id} size={52} kind="community" />
          <div class="grow">
            <div class="name truncate">{g().subject}</div>
            <div class="sub">Community · {plural(subs().length, "group")}{g().size ? ` · ${g().size} members` : ""}</div>
          </div>
          <Icon name="caret-right" size={16} class="faint" />
        </button>
        <Show when={announcement()}>
          {(a) => (
            <>
              <div class="side-section">Announcements</div>
              <Show when={state.chats[a().id]} fallback={<div class="empty">No announcements yet</div>}>
                {(c) => <ChatRow chat={c()} />}
              </Show>
            </>
          )}
        </Show>
        <div class="side-section">Groups you're in</div>
        <For each={subs().filter((s) => s.joined)} fallback={<div class="empty" style={{ padding: "16px" }}>You haven't joined any groups here yet.</div>}>
          {(s) => <Show when={state.chats[s.id]}>{(c) => <ChatRow chat={c()} />}</Show>}
        </For>
        <Show when={subs().some((s) => !s.joined)}>
          <div class="side-section">Groups you can join</div>
          <For each={subs().filter((s) => !s.joined)}>
            {(s) => (
              <div class="list-row" style={{ cursor: "default" }}>
                <Avatar id={s.id} size={44} kind="group" name={s.subject} />
                <div class="grow">
                  <div class="title truncate">{s.subject}</div>
                  <div class="sub truncate">{s.size ? plural(s.size, "member") : "Group"}{s.approval ? " · approval needed" : ""}</div>
                </div>
                <button class="btn sm" onClick={async () => {
                  await act("community", { op: { op: "joinSubgroup", id: g().id, group: s.id } });
                  if (state.chats[s.id]) void openChat(s.id);
                }}>{s.approval ? "Request" : "Join"}</button>
              </div>
            )}
          </For>
        </Show>
      </div>
    </>
  );
}

function CommunitiesView() {
  const current = () => (state.community ? state.groups[state.community] : undefined);
  return (
    <Show when={current()} fallback={
      <>
        <div class="side-head drag">
          <h1>Communities</h1>
          <IconButton class="no-drag" icon="plus" tip="New community" onClick={() => openModal("newCommunity")} />
        </div>
        <div class="side-list">
          <For each={derived.communities()} fallback={
            <div class="empty">
              <Icon name="users-three" />
              <strong>No communities yet</strong>
              Communities bring related groups together under one roof, with a shared announcements channel.
              <button class="btn primary sm" style={{ "margin-top": "12px" }} onClick={() => openModal("newCommunity")}>Create a community</button>
            </div>
          }>
            {(g) => {
              const subs = () => subgroupsOf(g.id);
              const unread = () => Object.values(state.chats).filter((c) => c.community === g.id && (c.unread > 0 || c.markedUnread)).length;
              return (
                <button class="chat-row" onClick={() => setState("community", g.id)}>
                  <Avatar id={g.id} size={48} kind="community" />
                  <div class="lines">
                    <div class="top"><span class="name truncate">{g.subject}</span></div>
                    <div class="bottom">
                      <div class="preview"><span class="truncate">{plural(subs().length, "group")} · {subs().filter((s) => s.joined).length} joined</span></div>
                      <div class="marks"><Show when={unread()}><span class="badge">{unread()}</span></Show><Icon name="caret-right" /></div>
                    </div>
                  </div>
                </button>
              );
            }}
          </For>
        </div>
      </>
    }>
      {(g) => <CommunityHome group={g()} />}
    </Show>
  );
}

export function Sidebar() {
  const [dragging, setDragging] = createSignal(false);
  const [width, setWidth] = createSignal(state.settings.sidebarWidth);
  createEffect(on(() => state.settings.sidebarWidth, (w) => !dragging() && setWidth(w)));
  const startResize = (e: PointerEvent) => {
    e.preventDefault();
    setDragging(true);
    const startX = e.clientX, startW = width();
    const move = (ev: PointerEvent) => setWidth(clamp(startW + ev.clientX - startX, 280, 520));
    const up = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.cursor = "";
      updateSettings({ sidebarWidth: Math.round(width()) });
    };
    document.body.style.cursor = "col-resize";
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  return (
    <aside class="sidebar" style={{ width: `${width()}px` }}>
      <Switch>
        <Match when={state.view === "chats"}><ChatsView /></Match>
        <Match when={state.view === "archived"}><ArchivedView /></Match>
        <Match when={state.view === "starred"}><StarredView /></Match>
        <Match when={state.view === "communities"}><CommunitiesView /></Match>
      </Switch>
      <Show when={state.demo}>
        <div class="banner demo" style={{ margin: "0 12px 12px" }}>
          <Icon name="sparkle" />
          Demo mode: nothing here is real
          <button onClick={() => void act("demoExit")}>Exit</button>
        </div>
      </Show>
      <div class={`sidebar-resize ${dragging() ? "dragging" : ""}`} onPointerDown={startResize} onDblClick={() => { setWidth(348); updateSettings({ sidebarWidth: 348 }); }} />
    </aside>
  );
}
