// Right-hand panel: contact/group details, in-chat search, starred messages.

import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onMount } from "solid-js";
import { call } from "../lib/ipc";
import { debounce, ephemeralLabel, fullStamp, muteLabel, phone, plural, shortDate } from "../lib/format";
import {
  act, chatName, contactName, derived, isAdmin, isMuted, jumpToMessage, myRole, openChat, openModal, setState, startChat,
  state,
} from "../lib/state";
import type { Chat, ContentOf, Group, Jid, Message } from "../lib/types";
import { Photo, saveMedia } from "./Bubble";
import { fileToAttachment, pickFiles } from "./Pickers";
import { MessageHit, starVersion } from "./Sidebar";
import { Avatar, Icon, IconButton, type MenuEntry, Switch as Toggle, openMenu } from "./ui";

function Head(props: { title: string; back?: () => void }) {
  return (
    <div class="panel-head drag">
      <Show when={props.back} fallback={<IconButton class="no-drag" icon="x" tip="Close" onClick={() => setState("panel", null)} />}>
        <IconButton class="no-drag" icon="arrow-left" tip="Back" onClick={props.back} />
      </Show>
      <h2>{props.title}</h2>
    </div>
  );
}

// ───────────────────────────── media gallery ─────────────────────────────

function Gallery(props: { chat: Jid; back: () => void }) {
  const [tab, setTab] = createSignal<"media" | "docs" | "links" | "audio">("media");
  const [list] = createResource(tab, (t) => call<Message[]>("gallery", { chat: props.chat, tab: t }).catch(() => []));
  const tabs: [ReturnType<typeof tab>, string][] = [["media", "Media"], ["docs", "Docs"], ["links", "Links"], ["audio", "Audio"]];
  const url = (m: Message) => (m.content.type === "text" ? /(https?:\/\/[^\s]+)/.exec(m.content.text)?.[1] ?? "" : "");
  return (
    <>
      <Head title="Media, links and docs" back={props.back} />
      <div class="seg">
        <For each={tabs}>{([id, label]) => <button class={tab() === id ? "on" : ""} onClick={() => setTab(id)}>{label}</button>}</For>
      </div>
      <div class="panel-body">
        <Show when={!list.loading} fallback={<div class="empty"><div class="spinner" /></div>}>
          <Show when={(list() ?? []).length} fallback={<div class="empty"><Icon name="images" /><strong>Nothing here yet</strong>{tab() === "media" ? "Photos and videos shared in this chat will show up here." : ""}</div>}>
            <Switch>
              <Match when={tab() === "media"}>
                <div class="media-grid">
                  <For each={list()}>{(m) => <Photo m={m} c={m.content as ContentOf<"image">} tile />}</For>
                </div>
              </Match>
              <Match when={tab() === "links"}>
                <For each={list()}>
                  {(m) => (
                    <button class="list-row" onClick={() => void jumpToMessage(m.id)}>
                      <span class="glyph"><Icon name="link-simple" /></span>
                      <div class="grow">
                        <div class="title truncate">{m.content.type === "text" && m.content.preview ? m.content.preview.title : url(m)}</div>
                        <div class="sub truncate">{url(m)} · {shortDate(m.ts)}</div>
                      </div>
                    </button>
                  )}
                </For>
              </Match>
              <Match when={true}>
                <For each={list()}>
                  {(m) => (
                    <button class="list-row" onClick={() => void jumpToMessage(m.id)} onContextMenu={(e) => openMenu(e, [{ label: "Save as…", icon: "download-simple", run: () => void saveMedia(m) }, { label: "Show in chat", icon: "chat-circle", run: () => void jumpToMessage(m.id) }])}>
                      <span class="glyph"><Icon name={tab() === "audio" ? (m.content.type === "audio" && m.content.voice ? "microphone" : "headphones") : "file-text"} /></span>
                      <div class="grow">
                        <div class="title truncate">{m.content.type === "document" ? m.content.name : m.content.type === "audio" && m.content.voice ? "Voice message" : "Audio"}</div>
                        <div class="sub truncate">{contactName(m.sender)} · {shortDate(m.ts)}</div>
                      </div>
                    </button>
                  )}
                </For>
              </Match>
            </Switch>
          </Show>
        </Show>
      </div>
    </>
  );
}

function MediaCard(props: { chat: Jid; open: () => void }) {
  const [list] = createResource(() => state.convo.messages.length, () => call<Message[]>("gallery", { chat: props.chat, tab: "media" }).catch(() => []));
  return (
    <div class="card">
      <div class="card-title"><span>Media, links and docs</span><button onClick={props.open}>See all</button></div>
      <Show when={(list() ?? []).length} fallback={<div class="hint" style={{ padding: "2px 12px 12px" }}>No media shared yet</div>}>
        <div class="media-strip">
          <For each={(list() ?? []).slice(0, 4)}>{(m) => <Photo m={m} c={m.content as ContentOf<"image">} tile />}</For>
        </div>
      </Show>
    </div>
  );
}

// ───────────────────────────── shared rows ─────────────────────────────

function CommonRows(props: { chat: Chat }) {
  const c = () => props.chat;
  return (
    <div class="card">
      <button class="set-row" onClick={() => setState("panel", "starred")}><Icon name="star" /><div class="grow s-title">Starred messages</div><Icon name="caret-right" size={15} class="faint" /></button>
      <div class="set-row">
        <Icon name={isMuted(c()) ? "bell-slash" : "bell"} />
        <div class="grow"><div class="s-title">Mute notifications</div><Show when={isMuted(c())}><div class="s-sub">{muteLabel(c().mutedUntil, state.now)}</div></Show></div>
        <Toggle on={isMuted(c())} onChange={(v) => (v ? openModal("mute", { chat: c().id }) : void act("mute", { chat: c().id, until: 0 }))} label="Mute" />
      </div>
      <Show when={!c().readOnly}>
        <button class="set-row" onClick={() => openModal("ephemeral", { chat: c().id })}><Icon name="timer" /><div class="grow"><div class="s-title">Disappearing messages</div><div class="s-sub">{ephemeralLabel(c().ephemeral)}</div></div><Icon name="caret-right" size={15} class="faint" /></button>
      </Show>
      <div class="set-row"><Icon name="lock" /><div class="grow"><div class="s-title">Encryption</div><div class="s-sub">Messages are end-to-end encrypted.</div></div></div>
    </div>
  );
}

// ───────────────────────────── contact info ─────────────────────────────

function ContactInfo(props: { chat: Chat; gallery: () => void }) {
  const id = () => props.chat.id;
  const c = () => state.contacts[id()];
  const self = () => id() === state.me?.id;
  const common = createMemo(() => Object.values(state.groups).filter((g) => !g.isCommunity && g.joined && g.participants.some((p) => p.id === id())));
  onMount(() => void call("avatar", { jid: id() }).catch(() => {}));
  return (
    <>
      <Head title="Contact info" />
      <div class="panel-body">
        <div class="profile">
          <Avatar id={id()} size={116} />
          <h3>{chatName(id())}<Show when={c()?.verifiedName}><Icon name="check-circle-fill" /></Show></h3>
          <div class="p-sub selectable">{phone(id())}{c()?.name && c()?.pushName ? ` · ~${c()!.pushName}` : ""}</div>
          <Show when={c()?.isBusiness}><div class="p-sub">Business account</div></Show>
          <div class="p-actions">
            <button class="tile-btn" onClick={() => setState("panel", "search")}><Icon name="magnifying-glass" />Search</button>
            <Show when={!self()}>
              <button class="tile-btn" onClick={() => openModal("editText", { title: c()?.name ? "Edit contact name" : "Save contact", label: "Name", value: c()?.name ?? c()?.pushName ?? "", onSave: (v: string) => act("saveContact", { jid: id(), name: v }) })}>
                <Icon name="user-plus" />{c()?.name ? "Edit" : "Save"}
              </button>
            </Show>
            <button class={`tile-btn ${props.chat.favorite ? "on" : ""}`} onClick={() => void act("favorite", { chat: id(), on: !props.chat.favorite })}>
              <Icon name={props.chat.favorite ? "heart-fill" : "heart"} />Favourite
            </button>
          </div>
        </div>
        <Show when={c()?.about}>
          <div class="card"><div class="card-title"><span>About</span></div><div class="card-text">{c()!.about}</div></div>
        </Show>
        <MediaCard chat={id()} open={props.gallery} />
        <CommonRows chat={props.chat} />
        <Show when={common().length && !self()}>
          <div class="card">
            <div class="card-title"><span>{plural(common().length, "group")} in common</span></div>
            <For each={common()}>
              {(g) => (
                <button class="member" onClick={() => void openChat(g.id)}>
                  <Avatar id={g.id} size={36} />
                  <div class="grow"><div class="m-name truncate">{g.subject}</div><div class="m-sub truncate">{g.participants.slice(0, 5).map((p) => contactName(p.id, { short: true }).replace(/^~/, "")).join(", ")}</div></div>
                </button>
              )}
            </For>
          </div>
        </Show>
        <div class="card">
          <Show when={!self()}>
            <button class="set-row danger" onClick={() => void act("block", { jid: id(), on: !c()?.blocked })}><Icon name="prohibit" /><div class="s-title">{c()?.blocked ? "Unblock" : "Block"} {contactName(id(), { you: false, short: true }).replace(/^~/, "")}</div></button>
          </Show>
          <button class="set-row danger" onClick={() => openModal("confirmClear", { chat: id() })}><Icon name="broom" /><div class="s-title">Clear messages</div></button>
          <button class="set-row danger" onClick={() => openModal("confirmDelete", { chat: id() })}><Icon name="trash" /><div class="s-title">Delete chat</div></button>
        </div>
      </div>
    </>
  );
}

// ───────────────────────────── group info ─────────────────────────────

function GroupInfo(props: { chat: Chat; group: Group; gallery: () => void }) {
  const g = () => props.group;
  const id = () => g().id;
  const admin = () => isAdmin(id());
  const canEdit = () => g().joined && (admin() || !g().locked);
  const [q, setQ] = createSignal("");
  const [showAll, setShowAll] = createSignal(false);
  const rank = { owner: 0, admin: 1, member: 2 } as const;
  const members = createMemo(() => {
    const s = q().trim().toLowerCase();
    return [...g().participants]
      .filter((p) => !s || contactName(p.id).toLowerCase().includes(s))
      .sort((a, b) => Number(b.id === state.me?.id) - Number(a.id === state.me?.id) || rank[a.role] - rank[b.role] || contactName(a.id).localeCompare(contactName(b.id)));
  });
  const edit = (title: string, label: string, value: string, op: string, extra: Record<string, unknown> = {}) =>
    openModal("editText", { title, label, value, ...extra, onSave: (v: string) => act("group", { op: { op, id: id(), text: v } }) });
  const icon = async () => {
    if (!canEdit()) return;
    const [file] = await pickFiles("image/png,image/jpeg,image/webp", false);
    if (file) await act("group", { op: { op: "icon", id: id(), file: await fileToAttachment(file) } });
  };
  const setting = (key: string, on: boolean) => void act("group", { op: { op: "setting", id: id(), key, on } });
  const memberMenu = (e: MouseEvent, jid: Jid, role: string) => {
    const me = jid === state.me?.id;
    const items: MenuEntry[] = [
      { label: `Message ${contactName(jid, { short: true }).replace(/^~/, "")}`, icon: "chat-circle", hidden: me, run: () => void startChat(jid) },
      { label: "View contact", icon: "user", hidden: me, run: () => openModal("contact", { jid }) },
      { sep: true, hidden: !admin() || me || role === "owner" },
      { label: role === "admin" ? "Dismiss as admin" : "Make group admin", icon: "crown", hidden: !admin() || me || role === "owner", run: () => void act("group", { op: { op: "participants", id: id(), action: role === "admin" ? "demote" : "promote", jids: [jid] } }) },
      { label: "Remove from group", icon: "user-minus", danger: true, hidden: !admin() || me || role === "owner", run: () => openModal("confirm", { title: `Remove ${contactName(jid)}?`, body: `They will no longer be able to send or receive messages in “${g().subject}”.`, action: "Remove", danger: true, onConfirm: () => void act("group", { op: { op: "participants", id: id(), action: "remove", jids: [jid] } }) }) },
    ];
    openMenu(e, items);
  };
  const community = () => (g().parent ? state.groups[g().parent!] : undefined);
  return (
    <>
      <Head title={g().isAnnouncement ? "Announcements" : "Group info"} />
      <div class="panel-body">
        <div class="profile">
          <div onClick={() => void icon()} title={canEdit() ? "Change group picture" : undefined}><Avatar id={id()} size={116} /></div>
          <h3>
            <span>{g().subject}</span>
            <Show when={canEdit() && !g().isAnnouncement}><IconButton icon="pencil-simple" small tip="Rename" onClick={() => edit("Group name", "Name", g().subject, "subject")} /></Show>
          </h3>
          <div class="p-sub">{g().isAnnouncement ? "Community announcements" : "Group"} · {plural(g().size || g().participants.length, "member")}</div>
          <Show when={community()}>
            {(c) => <button class="p-sub" style={{ color: "var(--accent)", "font-weight": 600 }} onClick={() => { setState({ view: "communities", community: c().id }); }}>Part of {c().subject}</button>}
          </Show>
          <div class="p-actions">
            <button class="tile-btn" onClick={() => setState("panel", "search")}><Icon name="magnifying-glass" />Search</button>
            <Show when={g().joined && (admin() || g().membersCanAdd) && !g().isAnnouncement}>
              <button class="tile-btn" onClick={() => openModal("addParticipants", { id: id() })}><Icon name="user-plus" />Add</button>
            </Show>
            <Show when={admin() && !g().isAnnouncement}>
              <button class="tile-btn" onClick={() => openModal("invite", { id: id() })}><Icon name="link-simple" />Invite</button>
            </Show>
            <button class={`tile-btn ${props.chat.favorite ? "on" : ""}`} onClick={() => void act("favorite", { chat: id(), on: !props.chat.favorite })}><Icon name={props.chat.favorite ? "heart-fill" : "heart"} />Favourite</button>
          </div>
        </div>
        <div class="card">
          <div class="card-title"><span>Description</span><Show when={canEdit()}><button onClick={() => edit("Group description", "Description", g().description ?? "", "description", { multiline: true, max: 2048, allowEmpty: true })}>{g().description ? "Edit" : "Add"}</button></Show></div>
          <div class="card-text">{g().description || <span class="faint">No description</span>}</div>
          <div class="hint" style={{ padding: "0 12px 10px" }}>Created {g().owner ? `by ${contactName(g().owner!)} ` : ""}on {fullStamp(g().createdAt).split(",").slice(0, 2).join(",")}</div>
        </div>
        <MediaCard chat={id()} open={props.gallery} />
        <CommonRows chat={props.chat} />
        <Show when={admin() && !g().isAnnouncement}>
          <div class="card">
            <div class="card-title"><span>Group permissions</span></div>
            <div class="set-row"><div class="grow"><div class="s-title">Only admins can send messages</div></div><Toggle on={g().announce} onChange={(v) => setting("announce", v)} /></div>
            <div class="set-row"><div class="grow"><div class="s-title">Only admins can edit group info</div></div><Toggle on={g().locked} onChange={(v) => setting("locked", v)} /></div>
            <div class="set-row"><div class="grow"><div class="s-title">Members can add others</div></div><Toggle on={g().membersCanAdd} onChange={(v) => setting("membersCanAdd", v)} /></div>
            <div class="set-row"><div class="grow"><div class="s-title">Approve new members</div><div class="s-sub">Admins must approve anyone who wants to join</div></div><Toggle on={g().approval} onChange={(v) => setting("approval", v)} /></div>
            <Show when={g().pending.length}>
              <button class="set-row" onClick={() => openModal("requests", { id: id() })}><Icon name="user-plus" /><div class="grow s-title">Requests to join</div><span class="badge">{g().pending.length}</span></button>
            </Show>
          </div>
        </Show>
        <div class="card">
          <div class="card-title"><span>{plural(g().participants.length, "member")}</span></div>
          <Show when={g().participants.length > 8}>
            <div class="pick-search" style={{ margin: "2px 6px 6px" }}><Icon name="magnifying-glass" /><input placeholder="Search members" value={q()} onInput={(e) => setQ(e.currentTarget.value)} /></div>
          </Show>
          <For each={showAll() || q() ? members() : members().slice(0, 10)}>
            {(p) => (
              <button class="member" onClick={(e) => memberMenu(e, p.id, p.role)} onContextMenu={(e) => memberMenu(e, p.id, p.role)}>
                <Avatar id={p.id} size={38} />
                <div class="grow"><div class="m-name truncate">{contactName(p.id)}</div><div class="m-sub truncate">{state.contacts[p.id]?.about || phone(p.id)}</div></div>
                <Show when={p.role !== "member"}><span class="role-tag">{p.role === "owner" ? "Owner" : "Admin"}</span></Show>
              </button>
            )}
          </For>
          <Show when={!showAll() && !q() && members().length > 10}>
            <button class="set-row" onClick={() => setShowAll(true)} style={{ color: "var(--accent)", "font-weight": 600 }}><Icon name="caret-down" style={{ color: "var(--accent)" }} />Show all {members().length} members</button>
          </Show>
        </div>
        <div class="card">
          <button class="set-row danger" onClick={() => openModal("confirmClear", { chat: id() })}><Icon name="broom" /><div class="s-title">Clear messages</div></button>
          <Show when={g().joined} fallback={<button class="set-row danger" onClick={() => openModal("confirmDelete", { chat: id() })}><Icon name="trash" /><div class="s-title">Delete group</div></button>}>
            <button class="set-row danger" onClick={() => openModal("confirmLeave", { chat: id() })}><Icon name="sign-out" /><div class="s-title">Exit group</div></button>
          </Show>
        </div>
        <Show when={myRole(id()) === null && g().joined}><div class="hint" style={{ padding: "0 20px" }}>You are viewing this group as a guest.</div></Show>
      </div>
    </>
  );
}

// ───────────────────────────── search & starred ─────────────────────────────

function SearchPanel(props: { chat: Jid }) {
  const [q, setQ] = createSignal("");
  const [term, setTerm] = createSignal("");
  const push = debounce((v: string) => setTerm(v), 160);
  createEffect(() => push(q()));
  const [hits] = createResource(term, (t) => (t.trim().length >= 2 ? call<Message[]>("search", { query: t, chat: props.chat }).catch(() => []) : []));
  return (
    <>
      <Head title="Search in chat" />
      <div class="pick-search" style={{ margin: "12px" }}>
        <Icon name="magnifying-glass" />
        <input ref={(el) => setTimeout(() => el.focus(), 60)} placeholder={`Search in ${chatName(props.chat)}`} value={q()} spellcheck={false} onInput={(e) => setQ(e.currentTarget.value)} onKeyDown={(e) => e.key === "Escape" && setState("panel", null)} />
      </div>
      <div class="panel-body">
        <For each={hits()} fallback={
          <div class="empty">
            <Icon name="magnifying-glass" />
            <Show when={term().trim().length >= 2 && !hits.loading} fallback={<><strong>Search this chat</strong>Find messages by any word they contain.</>}>
              <strong>No messages found</strong>
            </Show>
          </div>
        }>
          {(m) => <MessageHit m={m} query={q()} showChat={false} onPick={() => void jumpToMessage(m.id)} />}
        </For>
      </div>
    </>
  );
}

function StarredPanel(props: { chat: Jid }) {
  const [list] = createResource(starVersion, () => call<Message[]>("starred", { chat: props.chat }).catch(() => []));
  return (
    <>
      <Head title="Starred messages" back={() => setState("panel", "info")} />
      <div class="panel-body" style={{ "padding-top": "8px" }}>
        <For each={list()} fallback={<Show when={!list.loading}><div class="empty"><Icon name="star" /><strong>No starred messages</strong>Right-click a message and choose Star to keep it handy.</div></Show>}>
          {(m) => <MessageHit m={m} showChat={false} onPick={() => void jumpToMessage(m.id)} />}
        </For>
      </div>
    </>
  );
}

export function SidePanel() {
  const [gallery, setGallery] = createSignal(false);
  createEffect(on(() => [state.convo.chat, state.panel], () => setGallery(false)));
  const chat = () => derived.activeChat();
  return (
    <Show when={chat()}>
      {(c) => (
        <aside class="panel">
          <Switch>
            <Match when={state.panel === "search"}><SearchPanel chat={c().id} /></Match>
            <Match when={state.panel === "starred"}><StarredPanel chat={c().id} /></Match>
            <Match when={gallery()}><Gallery chat={c().id} back={() => setGallery(false)} /></Match>
            <Match when={c().kind !== "direct" && state.groups[c().id]}>
              {(g) => <GroupInfo chat={c()} group={g()} gallery={() => setGallery(true)} />}
            </Match>
            <Match when={true}><ContactInfo chat={c()} gallery={() => setGallery(true)} /></Match>
          </Switch>
        </aside>
      )}
    </Show>
  );
}
