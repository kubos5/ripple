// Command palette: jump to a chat, person or message, or run an action.

import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { call } from "../lib/ipc";
import { debounce, listTime, phone } from "../lib/format";
import {
  act, chatName, closeModal, contactName, derived, messageLabel, openChat, openModal, previewText, revealMessage, setState, startChat,
  state, updateSettings,
} from "../lib/state";
import type { Message } from "../lib/types";
import { Avatar, Icon, type IconName } from "./ui";

interface Entry {
  key: string;
  section: string;
  title: string;
  sub?: string;
  icon?: IconName;
  avatar?: string;
  hint?: string;
  run: () => void;
}

export function Palette() {
  const [q, setQ] = createSignal("");
  const [active, setActive] = createSignal(0);
  const [term, setTerm] = createSignal("");
  const push = debounce((v: string) => setTerm(v), 140);
  createEffect(() => push(q()));
  const [hits] = createResource(term, (t) => (t.trim().length >= 2 ? call<Message[]>("search", { query: t }).catch(() => []) : []));
  let listEl: HTMLDivElement | undefined;

  const close = () => closeModal();
  const go = (fn: () => void) => () => { closeModal(); fn(); };

  const actions = (): Entry[] => {
    const mod = state.platform === "macos" ? "⌘" : "Ctrl+";
    const dark = document.documentElement.dataset.theme === "dark";
    const list: Entry[] = [
      { key: "a-new", section: "Actions", title: "New chat", icon: "note-pencil", hint: `${mod}N`, run: () => openModal("newChat") },
      { key: "a-group", section: "Actions", title: "New group", icon: "users", run: () => openModal("newGroup") },
      { key: "a-comm", section: "Actions", title: "New community", icon: "users-three", run: () => openModal("newCommunity") },
      { key: "a-join", section: "Actions", title: "Join a group with a link", icon: "link-simple", run: () => openModal("joinGroup") },
      { key: "a-star", section: "Actions", title: "Starred messages", icon: "star", run: go(() => setState("view", "starred")) },
      { key: "a-arch", section: "Actions", title: "Archived chats", icon: "archive", run: go(() => setState("view", "archived")) },
      { key: "a-coms", section: "Actions", title: "Communities", icon: "users-three", run: go(() => setState("view", "communities")) },
      { key: "a-theme", section: "Actions", title: dark ? "Switch to light theme" : "Switch to dark theme", icon: dark ? "sun" : "moon", run: go(() => updateSettings({ theme: dark ? "light" : "dark" })) },
      { key: "a-set", section: "Actions", title: "Settings", icon: "gear-six", hint: `${mod},`, run: () => openModal("settings") },
      { key: "a-keys", section: "Actions", title: "Keyboard shortcuts", icon: "keyboard", run: () => openModal("settings", { tab: "shortcuts" }) },
    ];
    if (state.demo) {
      const demo = (key: string, title: string, what: string, icon: IconName) =>
        list.push({ key, section: "Demo", title, icon, run: go(() => void act("demoTrigger", { what })) });
      demo("d-call", "Demo: incoming voice call", "call", "phone-incoming");
      demo("d-video", "Demo: incoming video call", "videocall", "video-camera");
      demo("d-msg", "Demo: receive a message", "message", "chat-circle-dots");
      demo("d-type", "Demo: Maya starts typing", "typing", "dots-three");
      demo("d-off", "Demo: lose the connection", "offline", "wifi-slash");
      demo("d-pair", "Demo: show the QR pairing screen", "pairing", "qr-code");
      demo("d-code", "Demo: show the phone-number pairing screen", "pairingcode", "device-mobile");
    }
    return list;
  };

  const entries = createMemo<Entry[]>(() => {
    const s = q().trim().toLowerCase();
    const out: Entry[] = [];
    const chats = derived.all()
      .filter((c) => !s || chatName(c.id).toLowerCase().includes(s))
      .sort((a, b) => b.ts - a.ts)
      .slice(0, s ? 8 : 6);
    for (const c of chats) {
      out.push({
        key: `c-${c.id}`, section: s ? "Chats" : "Recent", title: chatName(c.id), avatar: c.id,
        sub: c.last ? previewText(c.last.text) : c.kind === "direct" ? phone(c.id) : "Group",
        hint: c.unread ? `${c.unread} unread` : undefined,
        run: go(() => { setState({ view: c.archived ? "archived" : "chats", query: "" }); void openChat(c.id); }),
      });
    }
    if (s) {
      const people = Object.values(state.contacts)
        .filter((c) => !c.isMe && !state.chats[c.id] && (c.name ?? "").toLowerCase().includes(s))
        .slice(0, 5);
      for (const c of people) out.push({ key: `p-${c.id}`, section: "Contacts", title: contactName(c.id), avatar: c.id, sub: c.about ?? phone(c.id), run: go(() => void startChat(c.id)) });
    }
    out.push(...actions().filter((a) => !s || a.title.toLowerCase().includes(s)));
    for (const m of (s.length >= 2 ? hits() ?? [] : []).slice(0, 12)) {
      out.push({
        key: `m-${m.chat}-${m.id}`, section: "Messages", title: chatName(m.chat), avatar: m.chat,
        sub: `${contactName(m.sender, { short: true }).replace(/^~/, "")}: ${messageLabel(m)}`, hint: listTime(m.ts, state.now),
        run: go(() => void revealMessage(m)),
      });
    }
    return out;
  });

  createEffect(() => { entries(); setActive(0); });
  createEffect(() => {
    active();
    listEl?.querySelector(".list-row.active")?.scrollIntoView({ block: "nearest" });
  });

  const onKey = (e: KeyboardEvent) => {
    const n = entries().length;
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === "ArrowDown") { e.preventDefault(); setActive((i) => (i + 1) % Math.max(1, n)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setActive((i) => (i - 1 + n) % Math.max(1, n)); }
    else if (e.key === "Enter") { e.preventDefault(); entries()[active()]?.run(); }
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));

  return (
    <Portal>
      <div class="scrim palette-scrim" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div class="palette" role="dialog" aria-label="Command palette">
          <div class="palette-input">
            <Icon name="magnifying-glass" />
            <input ref={(el) => setTimeout(() => el.focus(), 30)} placeholder="Search chats, people, messages and actions" value={q()} spellcheck={false} onInput={(e) => setQ(e.currentTarget.value)} />
            <span class="kbd">Esc</span>
          </div>
          <div class="palette-list" ref={listEl}>
            <For each={entries()} fallback={<div class="empty"><Icon name="magnifying-glass" /><strong>Nothing found</strong></div>}>
              {(e, i) => (
                <>
                  <Show when={i() === 0 || entries()[i() - 1].section !== e.section}><div class="side-section">{e.section}</div></Show>
                  <button class={`list-row ${i() === active() ? "active" : ""}`} onMouseMove={() => active() !== i() && setActive(i())} onClick={e.run}>
                    <Show when={e.avatar} fallback={<span class="glyph" style={{ width: "34px", height: "34px" }}><Icon name={e.icon ?? "lightning"} size={18} /></span>}>
                      <Avatar id={e.avatar!} size={34} />
                    </Show>
                    <div class="grow">
                      <div class="title truncate">{e.title}</div>
                      <Show when={e.sub}><div class="sub truncate">{e.sub}</div></Show>
                    </div>
                    <Show when={e.hint}><span class="faint" style={{ "font-size": "var(--fs-12)" }}>{e.hint}</span></Show>
                  </button>
                </>
              )}
            </For>
          </div>
          <div class="palette-foot">
            <span><span class="kbd">↑</span><span class="kbd">↓</span> Navigate</span>
            <span><span class="kbd">↵</span> Open</span>
            <span><span class="kbd">Esc</span> Close</span>
          </div>
        </div>
      </div>
    </Portal>
  );
}
