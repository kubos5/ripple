// Every dialog in the app. `ModalHost` renders whichever one `state.modal` names.

import { For, type JSX, Match, Show, Switch, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { encode } from "uqr";
import { call } from "../lib/ipc";
import { ephemeralLabel, fileSize, fullStamp, isGroupJid, phone, plural } from "../lib/format";
import {
  act, chatName, clearSelection, closeChat, closeModal, contactName, derived, isAdmin, openChat, openModal, send,
  setState, startChat, state, toast,
} from "../lib/state";
import type { Attachment, Group, Jid, Message, Receipt } from "../lib/types";
import { Palette } from "./Palette";
import { fileToAttachment } from "./Pickers";
import { Settings } from "./Settings";
import { Avatar, Confirm, Icon, IconButton, type IconName, Sheet, Switch as Toggle } from "./ui";

const prop = <T,>(key: string) => state.modal?.props?.[key] as T;

// ───────────────────────────── shared bits ─────────────────────────────

export function QR(props: { text: string; size?: number }) {
  const path = createMemo(() => {
    const { data, size } = encode(props.text, { ecc: "M", border: 0 });
    let d = "";
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (data[y][x]) d += `M${x} ${y}h1v1h-1z`;
    return { d, size };
  });
  return (
    <svg viewBox={`0 0 ${path().size} ${path().size}`} width={props.size} height={props.size} shape-rendering="crispEdges" role="img" aria-label="QR code">
      <path d={path().d} fill="#111314" />
    </svg>
  );
}

function SearchField(props: { value: string; onInput: (v: string) => void; placeholder: string; onEnter?: () => void }) {
  return (
    <div class="pick-search">
      <Icon name="magnifying-glass" />
      <input
        ref={(el) => setTimeout(() => el.focus(), 40)}
        placeholder={props.placeholder}
        value={props.value}
        spellcheck={false}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => e.key === "Enter" && props.onEnter?.()}
      />
    </div>
  );
}

/** People the user can pick: saved contacts first, then anyone they have chatted with. */
function people(exclude: Jid[] = []): Jid[] {
  const me = state.me?.id;
  const skip = new Set([...exclude, me]);
  const ids = new Set<Jid>();
  for (const c of Object.values(state.contacts)) if (!skip.has(c.id) && (c.name || state.chats[c.id])) ids.add(c.id);
  return [...ids].sort((a, b) => contactName(a, { you: false }).localeCompare(contactName(b, { you: false })));
}

function PeoplePicker(props: { picked: Jid[]; onChange: (v: Jid[]) => void; exclude?: Jid[]; max?: number }) {
  const [q, setQ] = createSignal("");
  const list = createMemo(() => {
    const s = q().trim().toLowerCase();
    return people(props.exclude).filter((j) => !s || contactName(j, { you: false }).toLowerCase().includes(s) || phone(j).replace(/\s/g, "").includes(s));
  });
  const toggle = (j: Jid) => {
    if (props.picked.includes(j)) props.onChange(props.picked.filter((x) => x !== j));
    else if (!props.max || props.picked.length < props.max) props.onChange([...props.picked, j]);
    else toast(`You can pick up to ${props.max}`);
  };
  return (
    <>
      <SearchField value={q()} onInput={setQ} placeholder="Search name or number" />
      <Show when={props.picked.length}>
        <div class="picked">
          <For each={props.picked}>
            {(j) => (
              <button class="chip on" onClick={() => toggle(j)}>
                <Avatar id={j} size={22} />{contactName(j, { you: false, short: true }).replace(/^~/, "")}<Icon name="x" size={12} />
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="pick-list">
        <For each={list()} fallback={<div class="empty"><Icon name="address-book" /><strong>No contacts found</strong></div>}>
          {(j) => (
            <button class={`pick-row ${props.picked.includes(j) ? "on" : ""}`} onClick={() => toggle(j)}>
              <Avatar id={j} size={40} />
              <div class="grow">
                <div class="title truncate">{contactName(j, { you: false })}</div>
                <div class="sub truncate">{state.contacts[j]?.about || phone(j)}</div>
              </div>
              <span class="check"><Icon name="check" /></span>
            </button>
          )}
        </For>
      </div>
    </>
  );
}

function useMessage(): () => Message | undefined {
  const chat = prop<Jid>("chat");
  const id = prop<string>("id");
  const [fetched] = createResource(() => call<{ message: Message | null }>("messageInfo", { chat, id }).then((r) => r.message ?? undefined).catch(() => undefined));
  return () => state.convo.messages.find((m) => m.id === id) ?? fetched();
}

function Choice(props: { options: { label: string; sub?: string; value: number; icon?: IconName }[]; value: number; onPick: (v: number) => void }) {
  return (
    <div class="col" style={{ gap: "2px", margin: "0 -8px" }}>
      <For each={props.options}>
        {(o) => (
          <button class={`pick-row ${props.value === o.value ? "on" : ""}`} onClick={() => props.onPick(o.value)}>
            <Show when={o.icon}>{(i) => <Icon name={i()} class="muted" />}</Show>
            <div class="grow">
              <div class="title">{o.label}</div>
              <Show when={o.sub}><div class="sub">{o.sub}</div></Show>
            </div>
            <span class="check"><Icon name="check" /></span>
          </button>
        )}
      </For>
    </div>
  );
}

// ───────────────────────────── starting chats ─────────────────────────────

function NewChat() {
  const [q, setQ] = createSignal(prop<string>("phone") ?? "");
  const [busy, setBusy] = createSignal(false);
  const list = createMemo(() => {
    const s = q().trim().toLowerCase();
    return people().filter((j) => !s || contactName(j, { you: false }).toLowerCase().includes(s) || phone(j).replace(/\s/g, "").includes(s.replace(/\s/g, "")));
  });
  const digits = () => q().replace(/[^\d]/g, "");
  const looksLikeNumber = () => digits().length >= 7 && /^[+\d\s()-]+$/.test(q().trim());
  const go = (jid: Jid) => { closeModal(); void startChat(jid); };
  const tryNumber = async () => {
    setBusy(true);
    const res = await act<{ exists: boolean; jid: Jid }>("checkNumber", { phone: digits() });
    setBusy(false);
    if (!res) return;
    if (res.exists) go(res.jid);
    else toast("That number isn't on WhatsApp", "error");
  };
  const action = (icon: IconName, label: string, run: () => void) => (
    <button class="pick-row" onClick={run}>
      <span class="glyph" style={{ width: "40px", height: "40px", "border-radius": "50%", display: "grid", "place-items": "center", background: "var(--accent-soft)", color: "var(--accent)" }}><Icon name={icon} /></span>
      <div class="title">{label}</div>
    </button>
  );
  return (
    <Sheet title="New chat" flush>
      <SearchField value={q()} onInput={setQ} placeholder="Search name or enter a phone number" onEnter={() => (list()[0] ? go(list()[0]) : looksLikeNumber() && void tryNumber())} />
      <div class="pick-list">
        <Show when={!q().trim()}>
          {action("users", "New group", () => openModal("newGroup"))}
          {action("users-three", "New community", () => openModal("newCommunity"))}
          {action("link-simple", "Join a group with a link", () => openModal("joinGroup"))}
          <Show when={state.me}>
            {(me) => (
              <button class="pick-row" onClick={() => go(me().id)}>
                <Avatar id={me().id} size={40} />
                <div class="grow"><div class="title">{me().pushName ?? "Me"} (You)</div><div class="sub">Message yourself</div></div>
              </button>
            )}
          </Show>
          <div class="side-section" style={{ "padding-left": "14px" }}>Contacts</div>
        </Show>
        <Show when={looksLikeNumber() && !list().length}>
          <button class="pick-row" disabled={busy()} onClick={() => void tryNumber()}>
            <span style={{ width: "40px", height: "40px", "border-radius": "50%", display: "grid", "place-items": "center", background: "var(--accent-soft)", color: "var(--accent)" }}>
              <Show when={busy()} fallback={<Icon name="chat-circle" />}><div class="spinner" /></Show>
            </span>
            <div class="grow"><div class="title">Message +{digits()}</div><div class="sub">Check if this number is on WhatsApp</div></div>
          </button>
        </Show>
        <For each={list()} fallback={<Show when={!looksLikeNumber()}><div class="empty"><Icon name="address-book" /><strong>No contacts found</strong>Type a full phone number with country code to message someone new.</div></Show>}>
          {(j) => (
            <button class="pick-row" onClick={() => go(j)}>
              <Avatar id={j} size={40} />
              <div class="grow">
                <div class="title truncate">{contactName(j, { you: false })}</div>
                <div class="sub truncate">{state.contacts[j]?.about || phone(j)}</div>
              </div>
            </button>
          )}
        </For>
      </div>
    </Sheet>
  );
}

function NewGroup() {
  const parent = prop<Jid | undefined>("parent");
  const [picked, setPicked] = createSignal<Jid[]>([]);
  const [step, setStep] = createSignal(0);
  const [subject, setSubject] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const create = async () => {
    setBusy(true);
    const res = await act<{ id: Jid }>("group", { op: { op: "create", subject: subject(), participants: picked(), parent: parent ?? null } });
    setBusy(false);
    if (!res) return;
    closeModal();
    setState("view", parent ? "communities" : "chats");
    void openChat(res.id);
  };
  return (
    <Sheet
      title={step() === 0 ? "Add group members" : "Name your group"}
      flush={step() === 0}
      foot={
        <>
          <Show when={step() === 1}><button class="btn ghost" onClick={() => setStep(0)}>Back</button></Show>
          <span class="grow" />
          <Show when={step() === 0} fallback={<button class="btn primary" disabled={!subject().trim() || busy()} onClick={() => void create()}>Create group</button>}>
            <button class="btn primary" disabled={!picked().length} onClick={() => setStep(1)}>Next</button>
          </Show>
        </>
      }
    >
      <Show when={step() === 0} fallback={
        <div class="form">
          <div>
            <label class="label">Group name</label>
            <input class="field" maxLength={100} value={subject()} ref={(el) => setTimeout(() => el.focus(), 40)} onInput={(e) => setSubject(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && subject().trim() && void create()} placeholder="Weekend plans" />
          </div>
          <div class="hint">{plural(picked().length, "member")}: {picked().map((j) => contactName(j, { you: false, short: true }).replace(/^~/, "")).join(", ")}{parent ? `. The group will be part of ${chatName(parent)}.` : ""}</div>
        </div>
      }>
        <PeoplePicker picked={picked()} onChange={setPicked} />
      </Show>
    </Sheet>
  );
}

function NewCommunity() {
  const [name, setName] = createSignal("");
  const [desc, setDesc] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const create = async () => {
    setBusy(true);
    const res = await act<{ id: Jid }>("community", { op: { op: "create", name: name(), description: desc() } });
    setBusy(false);
    if (!res) return;
    closeModal();
    setState({ view: "communities", community: res.id });
    toast("Community created", "success");
  };
  return (
    <Sheet title="New community" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!name().trim() || busy()} onClick={() => void create()}>Create community</button></>}>
      <div class="form">
        <p class="hint">A community groups related chats together and gives admins one announcements channel that reaches everyone.</p>
        <div>
          <label class="label">Community name</label>
          <input class="field" maxLength={100} ref={(el) => setTimeout(() => el.focus(), 40)} value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Neighbourhood, school, club…" />
        </div>
        <div>
          <label class="label">Description</label>
          <textarea class="field" rows={3} maxLength={2048} value={desc()} onInput={(e) => setDesc(e.currentTarget.value)} placeholder="What is this community for?" />
        </div>
      </div>
    </Sheet>
  );
}

function JoinGroup() {
  const [link, setLink] = createSignal("");
  const [info, setInfo] = createSignal<{ subject: string; size: number; description?: string } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const code = () => link().trim().replace(/^https?:\/\/chat\.whatsapp\.com\//i, "").split(/[?#/]/)[0];
  const look = async () => {
    setBusy(true);
    setInfo((await act<{ subject: string; size: number; description?: string }>("group", { op: { op: "inviteInfo", code: code() } })) ?? null);
    setBusy(false);
  };
  const join = async () => {
    setBusy(true);
    const res = await act<{ id: Jid }>("group", { op: { op: "join", code: code() } });
    setBusy(false);
    if (!res) return;
    closeModal();
    if (res.id && state.chats[res.id]) void openChat(res.id);
    else toast("Request sent. You'll join once an admin approves.", "success");
  };
  return (
    <Sheet title="Join a group" foot={
      <>
        <button class="btn ghost" onClick={closeModal}>Cancel</button>
        <Show when={info()} fallback={<button class="btn primary" disabled={code().length < 8 || busy()} onClick={() => void look()}>Look up</button>}>
          <button class="btn primary" disabled={busy()} onClick={() => void join()}>Join group</button>
        </Show>
      </>
    }>
      <div class="form">
        <div>
          <label class="label">Invite link</label>
          <input class="field" ref={(el) => setTimeout(() => el.focus(), 40)} value={link()} placeholder="https://chat.whatsapp.com/…" spellcheck={false}
            onInput={(e) => { setLink(e.currentTarget.value); setInfo(null); }} onKeyDown={(e) => e.key === "Enter" && code().length >= 8 && void (info() ? join() : look())} />
        </div>
        <Show when={info()}>
          {(g) => (
            <div class="card pad" style={{ margin: 0, display: "flex", gap: "12px", "align-items": "center" }}>
              <Avatar id={g().subject} name={g().subject} kind="group" size={48} src={null} />
              <div class="grow">
                <div style={{ "font-weight": 620 }}>{g().subject}</div>
                <div class="hint">{plural(g().size, "member")}{g().description ? ` · ${g().description}` : ""}</div>
              </div>
            </div>
          )}
        </Show>
      </div>
    </Sheet>
  );
}

// ───────────────────────────── chat-level choices ─────────────────────────────

function Mute() {
  const chat = prop<Jid>("chat");
  const [v, setV] = createSignal(8);
  const until = () => (v() === -1 ? -1 : Date.now() + v() * 3_600_000);
  return (
    <Sheet title="Mute notifications" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" onClick={() => { closeModal(); void act("mute", { chat, until: until() }); }}>Mute</button></>}>
      <p class="hint" style={{ "margin-bottom": "10px" }}>You'll still get notified when someone mentions you.</p>
      <Choice value={v()} onPick={setV} options={[{ label: "8 hours", value: 8 }, { label: "1 week", value: 168 }, { label: "Always", value: -1 }]} />
    </Sheet>
  );
}

function Ephemeral() {
  const chat = prop<Jid>("chat");
  const [v, setV] = createSignal(state.chats[chat]?.ephemeral ?? 0);
  const day = 86_400;
  return (
    <Sheet title="Disappearing messages" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" onClick={() => { closeModal(); if (v() !== (state.chats[chat]?.ephemeral ?? 0)) void act("ephemeral", { chat, secs: v() }); }}>Save</button></>}>
      <p class="hint" style={{ "margin-bottom": "10px" }}>New messages will disappear from this chat for everyone after the time you choose. Anyone can still save or screenshot them before then.</p>
      <Choice value={v()} onPick={setV} options={[0, day, 7 * day, 90 * day].map((s) => ({ label: ephemeralLabel(s), value: s }))} />
    </Sheet>
  );
}

function PinDuration() {
  const [v, setV] = createSignal(7 * 86_400);
  return (
    <Sheet title="Pin message" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" onClick={() => { const a = { chat: prop<Jid>("chat"), id: prop<string>("id"), secs: v() }; closeModal(); void act("pinMessage", a); }}>Pin</button></>}>
      <p class="hint" style={{ "margin-bottom": "10px" }}>Choose how long the message stays pinned. You can unpin it at any time.</p>
      <Choice value={v()} onPick={setV} options={[{ label: "24 hours", value: 86_400 }, { label: "7 days", value: 7 * 86_400 }, { label: "30 days", value: 30 * 86_400 }]} />
    </Sheet>
  );
}

function DeleteMessages() {
  const chat = prop<Jid>("chat");
  const ids = prop<string[]>("ids");
  const msgs = () => state.convo.messages.filter((m) => ids.includes(m.id));
  const canEveryone = () => {
    const list = msgs();
    if (!list.length || list.some((m) => m.content.type === "deleted" || m.content.type === "system")) return false;
    const fresh = list.every((m) => Date.now() - m.ts < 2 * 86_400_000);
    return fresh && (list.every((m) => m.fromMe) || (isGroupJid(chat) && isAdmin(chat)));
  };
  const run = (everyone: boolean) => {
    closeModal();
    clearSelection();
    void act("delete", { chat, ids, everyone });
  };
  return (
    <Sheet title={ids.length > 1 ? `Delete ${ids.length} messages?` : "Delete message?"}>
      <div class="col" style={{ gap: "8px" }}>
        <Show when={canEveryone()}>
          <button class="btn danger block lg" onClick={() => run(true)}>Delete for everyone</button>
        </Show>
        <button class={`btn block lg ${canEveryone() ? "" : "danger"}`} onClick={() => run(false)}>Delete for me</button>
        <button class="btn ghost block" onClick={closeModal}>Cancel</button>
      </div>
    </Sheet>
  );
}

function Forward() {
  const [q, setQ] = createSignal("");
  const [picked, setPicked] = createSignal<Jid[]>([]);
  const list = createMemo(() => {
    const s = q().trim().toLowerCase();
    return derived.all().filter((c) => !c.readOnly && (!s || chatName(c.id).toLowerCase().includes(s))).sort((a, b) => b.ts - a.ts);
  });
  const toggle = (j: Jid) =>
    setPicked((p) => (p.includes(j) ? p.filter((x) => x !== j) : p.length < 5 ? [...p, j] : (toast("You can forward to up to 5 chats at once"), p)));
  const go = async () => {
    const to = picked();
    const args = { chat: prop<Jid>("chat"), ids: prop<string[]>("ids"), to };
    closeModal();
    clearSelection();
    await act("forward", args);
    toast(`Forwarded to ${to.length === 1 ? chatName(to[0]) : plural(to.length, "chat")}`, "success");
    if (to.length === 1) void openChat(to[0]);
  };
  return (
    <Sheet title="Forward to…" flush foot={<><span class="grow hint truncate" style={{ "padding-left": "6px" }}>{picked().map((j) => chatName(j)).join(", ")}</span><button class="btn primary" disabled={!picked().length} onClick={() => void go()}><Icon name="paper-plane-right-fill" />Send</button></>}>
      <SearchField value={q()} onInput={setQ} placeholder="Search chats" />
      <div class="pick-list">
        <For each={list()}>
          {(c) => (
            <button class={`pick-row ${picked().includes(c.id) ? "on" : ""}`} onClick={() => toggle(c.id)}>
              <Avatar id={c.id} size={40} />
              <div class="grow"><div class="title truncate">{chatName(c.id)}</div><div class="sub truncate">{c.kind === "direct" ? (state.contacts[c.id]?.about || phone(c.id)) : plural(state.groups[c.id]?.participants.length ?? 0, "member")}</div></div>
              <span class="check"><Icon name="check" /></span>
            </button>
          )}
        </For>
      </div>
    </Sheet>
  );
}

// ───────────────────────────── message details ─────────────────────────────

function MessageInfo() {
  const chat = prop<Jid>("chat");
  const id = prop<string>("id");
  const msg = useMessage();
  const [res] = createResource(() => msg()?.status, () => call<{ receipts: Receipt[] }>("messageInfo", { chat, id }).then((r) => r.receipts).catch(() => []));
  const group = isGroupJid(chat);
  const read = () => (res() ?? []).filter((r) => r.read || r.played).sort((a, b) => b.read - a.read);
  const delivered = () => (res() ?? []).filter((r) => !r.read && !r.played && r.delivered);
  const waiting = () => {
    const seen = new Set((res() ?? []).map((r) => r.user));
    return (state.groups[chat]?.participants ?? []).map((p) => p.id).filter((j) => j !== state.me?.id && !seen.has(j));
  };
  const text = () => { const c = msg()?.content; return !c ? "" : c.type === "text" ? c.text : "caption" in c && c.caption ? c.caption : c.type; };
  const Row = (p: { jid: Jid; when?: number }) => (
    <div class="member">
      <Avatar id={p.jid} size={36} />
      <div class="grow"><div class="m-name truncate">{contactName(p.jid)}</div><Show when={p.when}><div class="m-sub">{fullStamp(p.when!)}</div></Show></div>
    </div>
  );
  return (
    <Sheet title="Message info">
      <Show when={msg()}>
        {(m) => (
          <>
            <div class="info-bubble truncate">{text()}</div>
            <Show when={group} fallback={
              <div class="col" style={{ gap: "2px" }}>
                <div class="set-row"><Icon name="checks" style={{ color: "var(--accent)" }} /><div class="grow"><div class="s-title">Read</div><div class="s-sub">{m().status === "read" || m().status === "played" ? (read()[0]?.read ? fullStamp(read()[0].read) : "Yes") : "Not yet"}</div></div></div>
                <div class="set-row"><Icon name="checks" /><div class="grow"><div class="s-title">Delivered</div><div class="s-sub">{["delivered", "read", "played"].includes(m().status) ? ((res() ?? [])[0]?.delivered ? fullStamp((res() ?? [])[0].delivered) : "Yes") : "Not yet"}</div></div></div>
                <div class="set-row"><Icon name="check" /><div class="grow"><div class="s-title">Sent</div><div class="s-sub">{fullStamp(m().ts)}</div></div></div>
              </div>
            }>
              <div class="receipt-list">
                <div class="card-title"><span>Read by {read().length}</span><Icon name="checks" size={16} style={{ color: "var(--accent)" }} /></div>
                <For each={read()} fallback={<div class="hint" style={{ padding: "4px 12px 8px" }}>No one yet</div>}>{(r) => <Row jid={r.user} when={r.read || r.played} />}</For>
                <Show when={delivered().length}>
                  <div class="card-title"><span>Delivered to {delivered().length}</span><Icon name="checks" size={16} /></div>
                  <For each={delivered()}>{(r) => <Row jid={r.user} when={r.delivered} />}</For>
                </Show>
                <Show when={waiting().length}>
                  <div class="card-title"><span>Not delivered yet · {waiting().length}</span></div>
                  <For each={waiting()}>{(j) => <Row jid={j} />}</For>
                </Show>
              </div>
            </Show>
          </>
        )}
      </Show>
    </Sheet>
  );
}

function ReactionList() {
  const msg = useMessage();
  const [tab, setTab] = createSignal("all");
  const groups = createMemo(() => {
    const map = new Map<string, number>();
    for (const r of msg()?.reactions ?? []) map.set(r.emoji, (map.get(r.emoji) ?? 0) + 1);
    return [...map.entries()].sort((a, b) => b[1] - a[1]);
  });
  const shown = () => (msg()?.reactions ?? []).filter((r) => tab() === "all" || r.emoji === tab());
  return (
    <Sheet title="Reactions" flush>
      <div class="chips" style={{ padding: "0 14px 10px" }}>
        <button class={`chip ${tab() === "all" ? "on" : ""}`} onClick={() => setTab("all")}>All {msg()?.reactions.length ?? 0}</button>
        <For each={groups()}>{([e, n]) => <button class={`chip ${tab() === e ? "on" : ""}`} onClick={() => setTab(e)}>{e} {n}</button>}</For>
      </div>
      <div class="pick-list">
        <For each={shown()}>
          {(r) => (
            <button class="pick-row" onClick={() => { if (r.sender === state.me?.id) { void act("react", { chat: msg()!.chat, id: msg()!.id, emoji: r.emoji }); if ((msg()?.reactions.length ?? 0) <= 1) closeModal(); } }}>
              <Avatar id={r.sender} size={40} />
              <div class="grow"><div class="title truncate">{contactName(r.sender)}</div><Show when={r.sender === state.me?.id}><div class="sub">Click to remove</div></Show></div>
              <span style={{ "font-size": "22px" }}>{r.emoji}</span>
            </button>
          )}
        </For>
      </div>
    </Sheet>
  );
}

function PollVotes() {
  const msg = useMessage();
  const poll = () => { const c = msg()?.content; return c?.type === "poll" ? c : undefined; };
  return (
    <Sheet title="Poll results">
      <Show when={poll()}>
        {(p) => (
          <>
            <div style={{ "font-weight": 640, "font-size": "var(--fs-15)", "margin-bottom": "10px" }}>{p().question}</div>
            <For each={p().options}>
              {(o) => (
                <div class="card" style={{ margin: "0 0 8px" }}>
                  <div class="card-title"><span style={{ color: "var(--text)", "font-size": "var(--fs-14)" }}>{o.name}</span><span>{plural(o.voters.length, "vote")}</span></div>
                  <For each={o.voters} fallback={<div class="hint" style={{ padding: "0 12px 10px" }}>No votes</div>}>
                    {(v) => <div class="member"><Avatar id={v} size={32} /><span class="m-name truncate">{contactName(v)}</span></div>}
                  </For>
                </div>
              )}
            </For>
          </>
        )}
      </Show>
    </Sheet>
  );
}

// ───────────────────────────── sending things ─────────────────────────────

function videoMeta(file: File): Promise<Partial<Attachment>> {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const v = document.createElement("video");
    const done = (meta: Partial<Attachment>) => { URL.revokeObjectURL(url); resolve(meta); };
    v.muted = true;
    v.preload = "auto";
    v.onerror = () => done({});
    v.onloadeddata = () => { v.currentTime = Math.min(0.4, (v.duration || 1) / 4); };
    v.onseeked = () => {
      const scale = Math.min(1, 320 / Math.max(v.videoWidth, v.videoHeight, 1));
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(v.videoWidth * scale);
      canvas.height = Math.round(v.videoHeight * scale);
      canvas.getContext("2d")?.drawImage(v, 0, 0, canvas.width, canvas.height);
      let thumb: string | null = null;
      try { thumb = canvas.toDataURL("image/jpeg", 0.7); } catch { /* tainted canvas */ }
      done({ width: v.videoWidth, height: v.videoHeight, duration: Math.round(v.duration || 0), thumb });
    };
    setTimeout(() => done({}), 6000);
    v.src = url;
  });
}

function audioMeta(file: File): Promise<Partial<Attachment>> {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const a = new Audio();
    const done = (d: number) => { URL.revokeObjectURL(url); resolve({ duration: Math.round(d) || 0 }); };
    a.onloadedmetadata = () => done(a.duration);
    a.onerror = () => done(0);
    setTimeout(() => done(0), 4000);
    a.src = url;
  });
}

function kindOf(file: File, asDocument: boolean) {
  if (asDocument) return "document";
  if (file.type.startsWith("image/") && !["image/gif", "image/svg+xml"].includes(file.type)) return "image";
  if (file.type.startsWith("video/")) return "video";
  if (file.type.startsWith("audio/")) return "audio";
  return "document";
}

function Attach() {
  const files = prop<File[]>("files");
  const [asDoc, setAsDoc] = createSignal(!!prop<boolean>("asDocument"));
  const [idx, setIdx] = createSignal(0);
  const [caption, setCaption] = createSignal("");
  const [once, setOnce] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const urls = files.map((f) => (f.type.startsWith("image/") || f.type.startsWith("video/") ? URL.createObjectURL(f) : ""));
  onCleanup(() => urls.forEach((u) => u && URL.revokeObjectURL(u)));
  const cur = () => files[idx()];
  const kind = (f: File) => kindOf(f, asDoc());
  const visual = (f: File) => ["image", "video"].includes(kind(f));
  const canOnce = () => files.length === 1 && visual(files[0]);
  const total = files.reduce((n, f) => n + f.size, 0);
  const sendAll = async () => {
    setBusy(true);
    const album = files.filter(visual).length > 1 ? `ALB${Date.now().toString(36)}` : null;
    try {
      for (let i = 0; i < files.length; i++) {
        const f = files[i];
        const k = kind(f);
        const file = { ...(await fileToAttachment(f)), ...(k === "video" ? await videoMeta(f) : k === "audio" ? await audioMeta(f) : {}) };
        await send(
          { type: "media", file, kind: k, caption: i === 0 ? caption().trim() : "", viewOnce: canOnce() && once(), album: visual(f) ? album : null },
          { chat: undefined, keepReply: i < files.length - 1 },
        );
      }
      closeModal();
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet
      title={files.length > 1 ? `Send ${files.length} files` : "Send file"}
      size="mid"
      foot={
        <>
          <Show when={files.some((f) => f.type.startsWith("image/") || f.type.startsWith("video/"))}>
            <button class="row" style={{ gap: "8px", "font-size": "var(--fs-13)" }} onClick={() => setAsDoc(!asDoc())}>
              <Toggle on={asDoc()} onChange={setAsDoc} label="Send as document" />Send as document
            </button>
          </Show>
          <span class="grow" />
          <button class="btn primary" disabled={busy()} onClick={() => void sendAll()}>
            <Show when={busy()} fallback={<Icon name="paper-plane-right-fill" />}><div class="spinner" /></Show>Send
          </button>
        </>
      }
    >
      <div class="attach-stage">
        <Switch>
          <Match when={kind(cur()) === "image"}><img src={urls[idx()]} alt="" /></Match>
          <Match when={kind(cur()) === "video"}><video src={urls[idx()]} controls /></Match>
          <Match when={true}>
            <div class="file">
              <Icon name={cur().type.startsWith("audio/") ? "file-audio" : "file-text"} />
              <strong style={{ color: "var(--text)" }}>{cur().name}</strong>
              {fileSize(cur().size)}
            </div>
          </Match>
        </Switch>
      </div>
      <Show when={files.length > 1}>
        <div class="attach-strip">
          <For each={files}>
            {(f, i) => (
              <button class={i() === idx() ? "on" : ""} onClick={() => setIdx(i())}>
                <Show when={f.type.startsWith("image/")} fallback={<Icon name={f.type.startsWith("video/") ? "video-camera" : "file-text"} />}><img src={urls[i()]} alt="" /></Show>
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="row" style={{ gap: "8px" }}>
        <input class="field" placeholder="Add a caption" value={caption()} maxLength={1024} ref={(el) => setTimeout(() => el.focus(), 60)}
          onInput={(e) => setCaption(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && !busy() && void sendAll()} />
        <Show when={canOnce()}>
          <IconButton icon="number-circle-one" tip="View once" tipPos="top" on={once()} onClick={() => setOnce(!once())} />
        </Show>
      </div>
      <Show when={total > 64 * 1024 * 1024}><p class="hint" style={{ "margin-top": "8px" }}>Large files can take a while to send. WhatsApp limits videos to about 100 MB and documents to 2 GB.</p></Show>
    </Sheet>
  );
}

function PollMaker() {
  const [q, setQ] = createSignal("");
  const [opts, setOpts] = createSignal(["", ""]);
  const [multi, setMulti] = createSignal(true);
  const filled = () => opts().map((o) => o.trim()).filter(Boolean);
  const valid = () => q().trim() && filled().length >= 2 && new Set(filled()).size === filled().length;
  const edit = (i: number, v: string) => {
    let next = opts().map((o, n) => (n === i ? v : o));
    if (next.every((o) => o.trim()) && next.length < 12) next = [...next, ""];
    setOpts(next);
  };
  const go = () => { const c = { type: "poll" as const, question: q().trim(), options: filled(), selectable: multi() ? 0 : 1 }; closeModal(); void send(c); };
  return (
    <Sheet title="Create poll" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!valid()} onClick={go}>Send poll</button></>}>
      <div class="form">
        <div>
          <label class="label">Question</label>
          <input class="field" maxLength={255} ref={(el) => setTimeout(() => el.focus(), 40)} value={q()} onInput={(e) => setQ(e.currentTarget.value)} placeholder="Ask something" />
        </div>
        <div>
          <label class="label">Options</label>
          <div class="col" style={{ gap: "6px" }}>
            <For each={opts()}>
              {(_, i) => (
                <div class="opt-row">
                  <input class="field" maxLength={100} value={opts()[i()]} placeholder={`Option ${i() + 1}`} onInput={(e) => edit(i(), e.currentTarget.value)} />
                  <Show when={opts().length > 2 && opts()[i()]}><IconButton icon="x" small tip="Remove" onClick={() => setOpts(opts().filter((_, n) => n !== i()))} /></Show>
                </div>
              )}
            </For>
          </div>
        </div>
        <div class="set-row" style={{ padding: "0 2px" }}>
          <div class="grow"><div class="s-title">Allow multiple answers</div></div>
          <Toggle on={multi()} onChange={setMulti} />
        </div>
      </div>
    </Sheet>
  );
}

function EventMaker() {
  const pad = (n: number) => String(n).padStart(2, "0");
  const soon = new Date(Date.now() + 86_400_000);
  const [name, setName] = createSignal("");
  const [desc, setDesc] = createSignal("");
  const [where, setWhere] = createSignal("");
  const [date, setDate] = createSignal(`${soon.getFullYear()}-${pad(soon.getMonth() + 1)}-${pad(soon.getDate())}`);
  const [time, setTime] = createSignal("18:00");
  const [end, setEnd] = createSignal("");
  const start = () => new Date(`${date()}T${time() || "00:00"}`).getTime();
  const valid = () => name().trim() && Number.isFinite(start());
  const go = () => {
    const endTs = end() ? new Date(`${date()}T${end()}`).getTime() : 0;
    const c = { type: "event" as const, name: name().trim(), description: desc().trim(), location: where().trim(), start: start(), end: endTs > start() ? endTs : 0 };
    closeModal();
    void send(c);
  };
  return (
    <Sheet title="Create event" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!valid()} onClick={go}>Send event</button></>}>
      <div class="form">
        <div><label class="label">Event name</label><input class="field" maxLength={100} ref={(el) => setTimeout(() => el.focus(), 40)} value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Dinner, match, meeting…" /></div>
        <div class="form-row">
          <div style={{ flex: 1.4 }}><label class="label">Date</label><input class="field" type="date" value={date()} onInput={(e) => setDate(e.currentTarget.value)} /></div>
          <div><label class="label">Starts</label><input class="field" type="time" value={time()} onInput={(e) => setTime(e.currentTarget.value)} /></div>
          <div><label class="label">Ends</label><input class="field" type="time" value={end()} onInput={(e) => setEnd(e.currentTarget.value)} /></div>
        </div>
        <div><label class="label">Location</label><input class="field" maxLength={200} value={where()} onInput={(e) => setWhere(e.currentTarget.value)} placeholder="Optional" /></div>
        <div><label class="label">Description</label><textarea class="field" rows={2} maxLength={2048} value={desc()} onInput={(e) => setDesc(e.currentTarget.value)} placeholder="Optional" /></div>
      </div>
    </Sheet>
  );
}

function ShareContact() {
  const [picked, setPicked] = createSignal<Jid[]>([]);
  return (
    <Sheet title="Share contacts" flush foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!picked().length} onClick={() => { const jids = picked(); closeModal(); void send({ type: "contacts", jids }); }}>Send</button></>}>
      <PeoplePicker picked={picked()} onChange={setPicked} max={10} />
    </Sheet>
  );
}

function LocationMaker() {
  const [lat, setLat] = createSignal("");
  const [lng, setLng] = createSignal("");
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const nums = () => ({ lat: Number(lat()), lng: Number(lng()) });
  const valid = () => lat().trim() !== "" && lng().trim() !== "" && Math.abs(nums().lat) <= 90 && Math.abs(nums().lng) <= 180;
  const locate = () => {
    if (!navigator.geolocation) return toast("Location isn't available here", "error");
    setBusy(true);
    navigator.geolocation.getCurrentPosition(
      (p) => { setLat(p.coords.latitude.toFixed(6)); setLng(p.coords.longitude.toFixed(6)); if (!name()) setName("My location"); setBusy(false); },
      () => { setBusy(false); toast("Couldn't get your location. Enter coordinates instead.", "error"); },
      { timeout: 8000 },
    );
  };
  const paste = (v: string) => {
    const m = /(-?\d{1,2}\.\d+)[,\s]+(-?\d{1,3}\.\d+)/.exec(v);
    if (m) { setLat(m[1]); setLng(m[2]); return true; }
    return false;
  };
  return (
    <Sheet title="Send location" foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!valid()} onClick={() => { const c = { type: "location" as const, ...nums(), name: name().trim() }; closeModal(); void send(c); }}>Send location</button></>}>
      <div class="form">
        <button class="btn block" disabled={busy()} onClick={locate}><Show when={busy()} fallback={<Icon name="map-pin" />}><div class="spinner" /></Show>Use my current location</button>
        <div class="form-row">
          <div><label class="label">Latitude</label><input class="field" inputmode="decimal" value={lat()} placeholder="54.4609" onInput={(e) => { if (!paste(e.currentTarget.value)) setLat(e.currentTarget.value); }} /></div>
          <div><label class="label">Longitude</label><input class="field" inputmode="decimal" value={lng()} placeholder="-3.0886" onInput={(e) => setLng(e.currentTarget.value)} /></div>
        </div>
        <div><label class="label">Place name</label><input class="field" maxLength={100} value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Optional" /></div>
        <p class="hint">Tip: paste coordinates copied from any map (for example “54.4609, -3.0886”) into the latitude box.</p>
      </div>
    </Sheet>
  );
}

// ───────────────────────────── people & groups ─────────────────────────────

function ContactCardModal() {
  const jid = prop<Jid>("jid");
  const c = () => state.contacts[jid];
  onMount(() => void call("avatar", { jid }).catch(() => {}));
  return (
    <Sheet title="Contact">
      <div class="profile" style={{ padding: "0 0 4px" }}>
        <Avatar id={jid} size={96} />
        <h3>{contactName(jid, { you: false })}<Show when={c()?.verifiedName}><Icon name="check-circle-fill" /></Show></h3>
        <div class="p-sub selectable">{phone(jid)}</div>
        <Show when={c()?.about}><div class="p-sub" style={{ "margin-top": "8px" }}>{c()!.about}</div></Show>
        <div class="p-actions">
          <button class="tile-btn" onClick={() => { closeModal(); void startChat(jid); }}><Icon name="chat-circle" />Message</button>
          <Show when={jid !== state.me?.id}>
            <button class="tile-btn" onClick={() => openModal("editText", { title: c()?.name ? "Edit contact name" : "Save contact", label: "Name", value: c()?.name ?? c()?.pushName ?? "", onSave: (v: string) => act("saveContact", { jid, name: v }) })}><Icon name="user-plus" />{c()?.name ? "Edit" : "Save"}</button>
            <button class="tile-btn" onClick={() => void act("block", { jid, on: !c()?.blocked })}><Icon name="prohibit" style={{ color: "var(--danger)" }} />{c()?.blocked ? "Unblock" : "Block"}</button>
          </Show>
        </div>
      </div>
    </Sheet>
  );
}

function EditText() {
  const [v, setV] = createSignal(prop<string>("value") ?? "");
  const [busy, setBusy] = createSignal(false);
  const max = prop<number>("max") ?? 100;
  const multiline = prop<boolean>("multiline");
  const save = async () => {
    setBusy(true);
    const onSave = prop<(v: string) => Promise<unknown>>("onSave");
    await onSave(v().trim());
    setBusy(false);
    closeModal();
  };
  return (
    <Sheet title={prop<string>("title")} foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={busy() || (!prop<boolean>("allowEmpty") && !v().trim())} onClick={() => void save()}>Save</button></>}>
      <label class="label">{prop<string>("label")}</label>
      <Show when={multiline} fallback={
        <input class="field" maxLength={max} value={v()} ref={(el) => setTimeout(() => { el.focus(); el.select(); }, 40)} onInput={(e) => setV(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && v().trim() && void save()} />
      }>
        <textarea class="field" rows={5} maxLength={max} value={v()} ref={(el) => setTimeout(() => el.focus(), 40)} onInput={(e) => setV(e.currentTarget.value)} />
      </Show>
      <div class="count-hint" style={{ "margin-top": "6px" }}>{v().length}/{max}</div>
    </Sheet>
  );
}

function AddParticipants() {
  const id = prop<Jid>("id");
  const [picked, setPicked] = createSignal<Jid[]>([]);
  const existing = () => state.groups[id]?.participants.map((p) => p.id) ?? [];
  return (
    <Sheet title="Add members" flush foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!picked().length} onClick={() => { const jids = picked(); closeModal(); void act("group", { op: { op: "participants", id, action: "add", jids } }); }}>Add {picked().length || ""}</button></>}>
      <PeoplePicker picked={picked()} onChange={setPicked} exclude={existing()} />
    </Sheet>
  );
}

function Invite() {
  const id = prop<Jid>("id");
  const [busy, setBusy] = createSignal(false);
  const fetchCode = async (reset: boolean) => {
    setBusy(true);
    await act("group", { op: { op: "invite", id, reset } });
    setBusy(false);
    if (reset) toast("The old link no longer works", "success");
  };
  onMount(() => { if (!state.groups[id]?.inviteCode) void fetchCode(false); });
  const link = () => (state.groups[id]?.inviteCode ? `https://chat.whatsapp.com/${state.groups[id]!.inviteCode}` : "");
  return (
    <Sheet title="Invite link">
      <div class="col" style={{ gap: "14px", "align-items": "center" }}>
        <Show when={link()} fallback={<div class="spinner" style={{ margin: "60px" }} />}>
          <div class="qr-box"><QR text={link()} size={188} /></div>
          <div class="field selectable truncate" style={{ display: "flex", "align-items": "center", "font-family": "var(--font-mono)", "font-size": "var(--fs-12)" }}>{link()}</div>
        </Show>
        <p class="hint" style={{ "text-align": "center" }}>Anyone with this link can {state.groups[id]?.approval ? "ask to" : ""} join “{chatName(id)}”. Only share it with people you trust.</p>
        <div class="row" style={{ gap: "8px" }}>
          <button class="btn primary" disabled={!link()} onClick={() => { void navigator.clipboard.writeText(link()); toast("Link copied", "success"); }}><Icon name="copy" />Copy link</button>
          <button class="btn" disabled={busy()} onClick={() => void fetchCode(true)}><Icon name="arrow-counter-clockwise" />Reset link</button>
        </div>
      </div>
    </Sheet>
  );
}

function Requests() {
  const id = prop<Jid>("id");
  const pending = () => state.groups[id]?.pending ?? [];
  const answer = (jids: Jid[], approve: boolean) => void act("group", { op: { op: "requestAction", id, jids, approve } });
  return (
    <Sheet title="Requests to join" flush foot={<Show when={pending().length > 1}><button class="btn primary" onClick={() => answer(pending(), true)}>Approve all</button></Show>}>
      <div class="pick-list">
        <For each={pending()} fallback={<div class="empty"><Icon name="user-plus" /><strong>No pending requests</strong></div>}>
          {(j) => (
            <div class="pick-row" style={{ cursor: "default" }}>
              <Avatar id={j} size={40} />
              <div class="grow"><div class="title truncate">{contactName(j)}</div><div class="sub">{phone(j)}</div></div>
              <IconButton icon="x" tip="Decline" onClick={() => answer([j], false)} />
              <button class="btn sm primary" onClick={() => answer([j], true)}>Approve</button>
            </div>
          )}
        </For>
      </div>
    </Sheet>
  );
}

function LinkGroups() {
  const id = prop<Jid>("id");
  const [picked, setPicked] = createSignal<Jid[]>([]);
  const eligible = () => Object.values(state.groups).filter((g: Group) => !g.isCommunity && !g.isAnnouncement && !g.parent && g.joined && isAdmin(g.id));
  const toggle = (j: Jid) => setPicked((p) => (p.includes(j) ? p.filter((x) => x !== j) : [...p, j]));
  return (
    <Sheet title="Add existing groups" flush foot={<><button class="btn ghost" onClick={closeModal}>Cancel</button><button class="btn primary" disabled={!picked().length} onClick={() => { const groups = picked(); closeModal(); void act("community", { op: { op: "link", id, groups } }); }}>Add to community</button></>}>
      <div class="pick-list">
        <For each={eligible()} fallback={<div class="empty"><Icon name="users" /><strong>No groups to add</strong>You can add groups you're an admin of that aren't already in a community.</div>}>
          {(g) => (
            <button class={`pick-row ${picked().includes(g.id) ? "on" : ""}`} onClick={() => toggle(g.id)}>
              <Avatar id={g.id} size={40} />
              <div class="grow"><div class="title truncate">{g.subject}</div><div class="sub">{plural(g.participants.length, "member")}</div></div>
              <span class="check"><Icon name="check" /></span>
            </button>
          )}
        </For>
      </div>
    </Sheet>
  );
}

function CommunityInfo() {
  const id = prop<Jid>("id");
  const g = () => state.groups[id];
  const subs = () => Object.values(state.groups).filter((x) => x.parent === id && !x.isAnnouncement);
  const admin = () => isAdmin(id);
  const edit = (title: string, label: string, value: string, op: string, extra: Record<string, unknown> = {}) =>
    openModal("editText", { title, label, value, ...extra, onSave: async (v: string) => { await act("group", { op: { op, id, text: v } }); openModal("communityInfo", { id }); } });
  return (
    <Sheet title="Community info">
      <Show when={g()}>
        {(c) => (
          <>
            <div class="profile" style={{ padding: "0 0 14px" }}>
              <Avatar id={id} size={92} kind="community" />
              <h3>{c().subject}</h3>
              <div class="p-sub">Community · {plural(subs().length, "group")}{c().size ? ` · ${plural(c().size, "member")}` : ""}</div>
            </div>
            <div class="card" style={{ margin: "0 0 8px" }}>
              <div class="card-title"><span>Description</span><Show when={admin()}><button onClick={() => edit("Community description", "Description", c().description ?? "", "description", { multiline: true, max: 2048, allowEmpty: true })}>Edit</button></Show></div>
              <div class="card-text">{c().description || <span class="faint">No description</span>}</div>
            </div>
            <div class="card" style={{ margin: "0 0 8px" }}>
              <div class="card-title"><span>Groups</span></div>
              <For each={subs()}>
                {(s) => (
                  <div class="member">
                    <Avatar id={s.id} size={34} kind="group" name={s.subject} />
                    <div class="grow"><div class="m-name truncate">{s.subject}</div><div class="m-sub">{s.joined ? "Joined" : "Not joined"}{s.size ? ` · ${plural(s.size, "member")}` : ""}</div></div>
                    <Show when={admin()}><IconButton icon="x" small tip="Remove from community" onClick={() => void act("community", { op: { op: "unlink", id, groups: [s.id] } })} /></Show>
                  </div>
                )}
              </For>
            </div>
            <Show when={admin()}>
              <div class="card" style={{ margin: 0 }}>
                <button class="set-row" onClick={() => edit("Community name", "Name", c().subject, "subject")}><Icon name="pencil-simple" /><div class="s-title">Rename community</div></button>
                <button class="set-row" onClick={() => openModal("linkGroups", { id })}><Icon name="plus" /><div class="s-title">Add existing groups</div></button>
                <button class="set-row danger" onClick={() => openModal("confirmDeactivate", { id })}><Icon name="trash" /><div class="s-title">Deactivate community</div></button>
              </div>
            </Show>
          </>
        )}
      </Show>
    </Sheet>
  );
}

// ───────────────────────────── host ─────────────────────────────

export function ModalHost(): JSX.Element {
  const type = () => state.modal?.type;
  return (
    <Show when={state.modal} keyed>
      {(_m) => (
        <Switch>
          <Match when={type() === "settings"}><Settings /></Match>
          <Match when={type() === "palette"}><Palette /></Match>
          <Match when={type() === "newChat"}><NewChat /></Match>
          <Match when={type() === "newGroup"}><NewGroup /></Match>
          <Match when={type() === "newCommunity"}><NewCommunity /></Match>
          <Match when={type() === "joinGroup"}><JoinGroup /></Match>
          <Match when={type() === "mute"}><Mute /></Match>
          <Match when={type() === "ephemeral"}><Ephemeral /></Match>
          <Match when={type() === "pin"}><PinDuration /></Match>
          <Match when={type() === "delete"}><DeleteMessages /></Match>
          <Match when={type() === "forward"}><Forward /></Match>
          <Match when={type() === "messageInfo"}><MessageInfo /></Match>
          <Match when={type() === "reactions"}><ReactionList /></Match>
          <Match when={type() === "pollVotes"}><PollVotes /></Match>
          <Match when={type() === "attach"}><Attach /></Match>
          <Match when={type() === "poll"}><PollMaker /></Match>
          <Match when={type() === "event"}><EventMaker /></Match>
          <Match when={type() === "shareContact"}><ShareContact /></Match>
          <Match when={type() === "location"}><LocationMaker /></Match>
          <Match when={type() === "contact"}><ContactCardModal /></Match>
          <Match when={type() === "editText"}><EditText /></Match>
          <Match when={type() === "addParticipants"}><AddParticipants /></Match>
          <Match when={type() === "invite"}><Invite /></Match>
          <Match when={type() === "requests"}><Requests /></Match>
          <Match when={type() === "linkGroups"}><LinkGroups /></Match>
          <Match when={type() === "communityInfo"}><CommunityInfo /></Match>
          <Match when={type() === "confirmClear"}>
            <Confirm title="Clear this chat?" body="All messages will be removed from this device and your other linked devices. The chat itself stays in your list." action="Clear messages" danger onConfirm={() => void act("clearChat", { chat: prop<Jid>("chat") })} />
          </Match>
          <Match when={type() === "confirmDelete"}>
            <Confirm title={`Delete chat with ${chatName(prop<Jid>("chat"))}?`} body="The chat and its messages will be removed from your devices." action="Delete chat" danger onConfirm={() => void act("deleteChat", { chat: prop<Jid>("chat") })} />
          </Match>
          <Match when={type() === "confirmLeave"}>
            <Confirm title={`Exit “${chatName(prop<Jid>("chat"))}”?`} body="You'll stop receiving messages from this group. Only admins will be told that you left." action="Exit group" danger onConfirm={() => void act("group", { op: { op: "leave", id: prop<Jid>("chat") } })} />
          </Match>
          <Match when={type() === "confirmDeactivate"}>
            <Confirm title="Deactivate community?" body="The community and its announcements channel will be removed. Its groups will stay, but will no longer be linked together." action="Deactivate" danger onConfirm={() => { setState("community", null); void act("community", { op: { op: "deactivate", id: prop<Jid>("id") } }); }} />
          </Match>
          <Match when={type() === "confirmLogout"}>
            <Confirm title="Log out of Ripple?" body="This device will be unlinked from your WhatsApp account and your message history will be removed from this computer." action="Log out" danger onConfirm={() => { closeChat(); void act(state.demo ? "demoExit" : "logout"); }} />
          </Match>
          <Match when={type() === "confirm"}>
            <Confirm title={prop<string>("title")} body={prop<string>("body")} action={prop<string>("action")} danger={prop<boolean>("danger")} onConfirm={() => prop<() => void>("onConfirm")()} />
          </Match>
        </Switch>
      )}
    </Show>
  );
}

