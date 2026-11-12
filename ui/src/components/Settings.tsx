// The settings window.

import { For, type JSX, Match, Show, Switch, createResource, createSignal } from "solid-js";
import { call, inTauri } from "../lib/ipc";
import { FONTS, loadAllFonts } from "../lib/fonts";
import { ephemeralLabel, fileSize, phone, plural } from "../lib/format";
import { act, contactName, openModal, state, toast, updateSettings } from "../lib/state";
import type { Privacy } from "../lib/types";
import { fileToAttachment, pickFiles } from "./Pickers";
import { Avatar, Icon, IconButton, type IconName, Sheet, Switch as Toggle } from "./ui";

type Tab = "profile" | "appearance" | "notifications" | "privacy" | "chats" | "storage" | "shortcuts" | "account";

const TABS: { id: Tab; label: string; icon: IconName }[] = [
  { id: "profile", label: "Profile", icon: "user-circle" },
  { id: "appearance", label: "Appearance", icon: "palette" },
  { id: "notifications", label: "Notifications", icon: "bell" },
  { id: "privacy", label: "Privacy", icon: "lock" },
  { id: "chats", label: "Chats & media", icon: "chat-circle" },
  { id: "storage", label: "Storage", icon: "hard-drives" },
  { id: "shortcuts", label: "Shortcuts", icon: "keyboard" },
  { id: "account", label: "Account", icon: "device-mobile" },
];

// The default accent follows the theme: white in dark mode, black in light mode.
const ACCENTS: [string, string, string][] = [
  ["mono", "Default", "var(--text)"], ["tidal", "Tidal", "#2fd0a5"], ["coral", "Coral", "#ff7a59"],
  ["amber", "Amber", "#f5a623"], ["sky", "Sky", "#3fb1f5"], ["rose", "Rose", "#f0629e"],
];

function Row(props: { title: string; sub?: string; icon?: IconName; children?: JSX.Element; onClick?: () => void; danger?: boolean }) {
  const body = (
    <>
      <Show when={props.icon}>{(i) => <Icon name={i()} />}</Show>
      <div class="grow">
        <div class="s-title">{props.title}</div>
        <Show when={props.sub}><div class="s-sub">{props.sub}</div></Show>
      </div>
      <div class="s-val">{props.children}</div>
    </>
  );
  return (
    <Show when={props.onClick} fallback={<div class="set-row">{body}</div>}>
      <button class={`set-row ${props.danger ? "danger" : ""}`} onClick={props.onClick}>{body}</button>
    </Show>
  );
}

function Check(props: { title: string; sub?: string; on: boolean; set: (v: boolean) => void; disabled?: boolean }) {
  return (
    <Row title={props.title} sub={props.sub}>
      <Toggle on={props.on} onChange={props.set} label={props.title} disabled={props.disabled} />
    </Row>
  );
}

function Select<T extends string | number>(props: { value: T; options: [T, string][]; set: (v: T) => void }) {
  return (
    <select
      class="select"
      value={String(props.value)}
      onChange={(e) => {
        const raw = e.currentTarget.value;
        props.set((typeof props.value === "number" ? Number(raw) : raw) as T);
      }}
    >
      <For each={props.options}>{([v, label]) => <option value={String(v)}>{label}</option>}</For>
    </select>
  );
}

function ProfilePane() {
  const me = () => state.me;
  const edit = (title: string, label: string, value: string, op: string, extra: Record<string, unknown> = {}) =>
    openModal("editText", { title, label, value, ...extra, onSave: async (v: string) => { await act("profile", { op: { op, text: v } }); openModal("settings", { tab: "profile" }); } });
  const photo = async () => {
    const [file] = await pickFiles("image/png,image/jpeg,image/webp", false);
    if (file && (await act("profile", { op: { op: "photo", file: await fileToAttachment(file) } })) !== undefined) toast("Profile photo updated", "success");
  };
  return (
    <Show when={me()}>
      {(m) => (
        <>
          <div class="card me-card">
            <button class="avatar-edit" onClick={() => void photo()} aria-label="Change profile photo">
              <Avatar id={m().id} size={84} name={m().pushName ?? "Me"} />
              <span class="cam"><Icon name="camera" /></span>
            </button>
            <div class="grow">
              <div class="display" style={{ "font-size": "var(--fs-20)" }}>{m().pushName ?? "Me"}</div>
              <div class="muted selectable">{phone(m().id)}</div>
              <Show when={m().avatar}>
                <button style={{ color: "var(--danger)", "font-size": "var(--fs-12)", "margin-top": "6px", "font-weight": 600 }} onClick={() => void act("profile", { op: { op: "photo", file: null } })}>Remove photo</button>
              </Show>
            </div>
          </div>
          <div class="card">
            <Row icon="user" title="Name" sub={m().pushName ?? "Not set"} onClick={() => edit("Your name", "Name", m().pushName ?? "", "name", { max: 25 })}><Icon name="pencil-simple" size={16} /></Row>
            <Row icon="info" title="About" sub={m().about || "Not set"} onClick={() => edit("About", "About", m().about ?? "", "about", { max: 139, allowEmpty: true })}><Icon name="pencil-simple" size={16} /></Row>
          </div>
          <p class="hint" style={{ padding: "4px 6px" }}>Your name and about are visible to people you chat with, depending on your privacy settings.</p>
        </>
      )}
    </Show>
  );
}

function AppearancePane() {
  const s = () => state.settings;
  loadAllFonts();
  const light = { "--side": "#f3f3f1", "--main": "#ffffff", "--in": "#e9e9e7", "--out": "#d6d6d4" };
  const dark = { "--side": "#1b1c1e", "--main": "#121314", "--in": "#2a2b2e", "--out": "#46474b" };
  const mock = (colors: Record<string, string>, half = false) => (
    <div class={`mock ${half ? "half" : ""}`} style={colors}><i /><b><span /><span /></b></div>
  );
  const themes: ["system" | "light" | "dark", string, () => JSX.Element][] = [
    ["system", "System", () => <>{mock(light)}{mock(dark, true)}</>],
    ["light", "Light", () => mock(light)],
    ["dark", "Dark", () => mock(dark)],
  ];
  return (
    <>
      <h3>Theme</h3>
      <div class="card">
        <div class="theme-pick">
          <For each={themes}>
            {([id, label, thumb]) => (
              <button class={s().theme === id ? "on" : ""} onClick={() => updateSettings({ theme: id })}>
                <div class="thumb">{thumb()}</div>
                {label}
              </button>
            )}
          </For>
        </div>
      </div>
      <h3>Accent colour</h3>
      <div class="card">
        <div class="swatches">
          <For each={ACCENTS}>
            {([id, label, color]) => (
              <button class={`swatch ${s().accent === id ? "on" : ""} ${id}`} style={{ "--c": color }} title={label} aria-label={label} onClick={() => updateSettings({ accent: id })}>
                <Icon name="check" />
              </button>
            )}
          </For>
        </div>
      </div>
      <h3>Font</h3>
      <div class="card">
        <div class="font-pick">
          <For each={FONTS}>
            {(f) => (
              <button class={s().font === f.id ? "on" : ""} onClick={() => updateSettings({ font: f.id })}>
                <span class="sample" style={{ "font-family": f.sample }}>Aa</span>
                {f.label}
              </button>
            )}
          </For>
        </div>
      </div>
      <h3>Layout</h3>
      <div class="card">
        <Row title="Text size" sub={`${Math.round(s().textScale * 100)}%`}>
          <span style={{ "font-size": "11px" }}>A</span>
          <input class="range" type="range" min="0.85" max="1.25" step="0.05" value={s().textScale} onInput={(e) => updateSettings({ textScale: Number(e.currentTarget.value) })} />
          <span style={{ "font-size": "17px" }}>A</span>
        </Row>
        <Check title="Compact chat list" sub="Fit more conversations on screen" on={s().compact} set={(v) => updateSettings({ compact: v })} />
      </div>
    </>
  );
}

function NotificationsPane() {
  const s = () => state.settings;
  return (
    <>
      <div class="card">
        <Check title="Desktop notifications" sub="Show a notification when a message arrives" on={s().notifications} set={(v) => updateSettings({ notifications: v })} />
        <Check title="Show message preview" sub="Include the sender and text in notifications" on={s().notificationPreview} set={(v) => updateSettings({ notificationPreview: v })} disabled={!s().notifications} />
        <Check title="Play a sound" on={s().notificationSound} set={(v) => updateSettings({ notificationSound: v })} disabled={!s().notifications} />
      </div>
      <p class="hint" style={{ padding: "4px 6px" }}>Muted and archived chats never notify you, even when someone mentions you.</p>
    </>
  );
}

function PrivacyPane() {
  const p = () => state.privacy;
  const set = (patch: Partial<Privacy>) => void act("setPrivacy", { privacy: { ...p(), ...patch } });
  const who: [string, string][] = [["all", "Everyone"], ["contacts", "My contacts"], ["none", "Nobody"]];
  const blocked = () => Object.values(state.contacts).filter((c) => c.blocked);
  const day = 86_400;
  return (
    <>
      <h3>Who can see my personal info</h3>
      <div class="card">
        <Row title="Last seen"><Select value={p().lastSeen} set={(v) => set({ lastSeen: v })} options={who} /></Row>
        <Row title="Online status"><Select value={p().online} set={(v) => set({ online: v })} options={[["all", "Everyone"], ["match_last_seen", "Same as last seen"]]} /></Row>
        <Row title="Profile photo"><Select value={p().profilePhoto} set={(v) => set({ profilePhoto: v })} options={who} /></Row>
        <Row title="About"><Select value={p().about} set={(v) => set({ about: v })} options={who} /></Row>
        <Check title="Read receipts" sub="If turned off, you won't send or receive read receipts. Group chats always send them." on={p().readReceipts} set={(v) => set({ readReceipts: v })} />
      </div>
      <h3>Messages</h3>
      <div class="card">
        <Row title="Who can add me to groups"><Select value={p().groupAdd} set={(v) => set({ groupAdd: v })} options={[["all", "Everyone"], ["contacts", "My contacts"]]} /></Row>
        <Row title="Default message timer" sub="Start new chats with disappearing messages">
          <Select value={p().defaultEphemeral} set={(v) => set({ defaultEphemeral: v })} options={[0, day, 7 * day, 90 * day].map((x) => [x, ephemeralLabel(x)] as [number, string])} />
        </Row>
      </div>
      <h3>Blocked contacts</h3>
      <div class="card">
        <For each={blocked()} fallback={<div class="hint" style={{ padding: "12px" }}>You haven't blocked anyone.</div>}>
          {(c) => (
            <div class="member">
              <Avatar id={c.id} size={34} />
              <div class="grow"><div class="m-name truncate">{contactName(c.id)}</div><div class="m-sub">{phone(c.id)}</div></div>
              <button class="btn sm" onClick={() => void act("block", { jid: c.id, on: false })}>Unblock</button>
            </div>
          )}
        </For>
      </div>
    </>
  );
}

function ChatsPane() {
  const s = () => state.settings;
  return (
    <>
      <h3>Writing</h3>
      <div class="card">
        <Check title="Enter sends the message" sub={s().enterSends ? "Shift + Enter adds a new line" : `${state.platform === "macos" ? "⌘" : "Ctrl"} + Enter sends`} on={s().enterSends} set={(v) => updateSettings({ enterSends: v })} />
        <Check title="Check spelling" on={s().spellcheck} set={(v) => updateSettings({ spellcheck: v })} />
        <Check title="Link previews" sub="Ripple fetches the page to build a preview for links you type" on={s().linkPreviews} set={(v) => updateSettings({ linkPreviews: v })} />
      </div>
      <h3>Download automatically</h3>
      <div class="card">
        <Check title="Photos and stickers" on={s().autoImages} set={(v) => updateSettings({ autoImages: v })} />
        <Check title="Voice messages and audio" on={s().autoAudio} set={(v) => updateSettings({ autoAudio: v })} />
        <Check title="Videos" on={s().autoVideo} set={(v) => updateSettings({ autoVideo: v })} />
        <Check title="Documents" on={s().autoDocuments} set={(v) => updateSettings({ autoDocuments: v })} />
      </div>
      <Show when={inTauri}>
        <h3>Desktop</h3>
        <div class="card">
          <Check title={state.platform === "macos" ? "Keep running when closed" : "Keep running in the tray"} sub={state.platform === "macos" ? "Closing the window keeps Ripple connected; click its Dock icon to reopen it" : "Closing the window keeps Ripple connected in the background"} on={s().closeToTray} set={(v) => updateSettings({ closeToTray: v })} />
          <Check title="Open at login" on={s().autostart} set={(v) => updateSettings({ autostart: v })} />
        </div>
      </Show>
    </>
  );
}

interface Usage { media: number; database: number; messages: number; chats: number }

function StoragePane() {
  const [usage, { refetch }] = createResource(() => call<Usage>("storage").catch(() => null));
  const [busy, setBusy] = createSignal(false);
  const total = () => (usage()?.media ?? 0) + (usage()?.database ?? 0);
  const clear = async () => {
    setBusy(true);
    const res = await act<{ freed: number }>("clearCache");
    setBusy(false);
    if (res) toast(res.freed ? `Freed ${fileSize(res.freed)}` : "Nothing to clear", "success");
    void refetch();
  };
  return (
    <Show when={usage()} fallback={<div class="empty"><div class="spinner" /></div>}>
      {(u) => (
        <>
          <div class="card">
            <Row title="Ripple is using" sub={`${plural(u().messages, "message")} in ${plural(u().chats, "chat")}`}><strong style={{ color: "var(--text)", "font-size": "var(--fs-15)" }}>{fileSize(total())}</strong></Row>
            <div class="usage-bar"><i style={{ width: `${total() ? (u().media / total()) * 100 : 0}%` }} /><i style={{ width: `${total() ? (u().database / total()) * 100 : 0}%` }} /></div>
            <Row icon="images" title="Photos, videos and files"><span>{fileSize(u().media)}</span></Row>
            <Row icon="database" title="Messages and search index"><span>{fileSize(u().database)}</span></Row>
          </div>
          <div class="card">
            <Row icon="broom" title="Clear downloaded media" sub="Removes files you received. They can be downloaded again while WhatsApp still has them.">
              <button class="btn sm" disabled={busy()} onClick={() => void clear()}>{busy() ? "Clearing…" : "Clear"}</button>
            </Row>
          </div>
        </>
      )}
    </Show>
  );
}

function ShortcutsPane() {
  const mod = state.platform === "macos" ? "⌘" : "Ctrl";
  const alt = state.platform === "macos" ? "⌥" : "Alt";
  const list: [string, string[]][] = [
    ["Jump to a chat, person or message", [mod, "K"]],
    ["New chat", [mod, "N"]],
    ["New group", [mod, "⇧", "N"]],
    ["Search all chats", [mod, "⇧", "F"]],
    ["Search in this chat", [mod, "F"]],
    ["Next / previous chat", [alt, "↓ ↑"]],
    ["Chat details", [mod, "⇧", "I"]],
    ["Mark chat as unread", [mod, "⇧", "U"]],
    ["Edit your last message", ["↑"]],
    ["Bold / italic / strikethrough", [mod, "B / I / ⇧X"]],
    ["Settings", [mod, ","]],
    ["Close chat, panel or dialog", ["Esc"]],
  ];
  return (
    <div class="card">
      <For each={list}>
        {([label, keys]) => (
          <div class="set-row">
            <div class="grow s-title">{label}</div>
            <div class="s-val"><For each={keys}>{(k) => <span class="kbd">{k}</span>}</For></div>
          </div>
        )}
      </For>
    </div>
  );
}

function AccountPane() {
  return (
    <>
      <div class="card">
        <Row icon="device-mobile" title={state.demo ? "Demo mode" : "Linked device"} sub={state.demo ? "You're exploring Ripple with made-up conversations. Nothing is sent anywhere." : `Linked to ${state.me ? phone(state.me.id) : "your phone"}. Your phone doesn't need to stay online.`} />
        <Row icon="shield-check" title="End-to-end encrypted" sub="Messages are encrypted on this device with the Signal protocol before they leave it." />
      </div>
      <div class="card">
        <Row icon="sign-out" danger title={state.demo ? "Exit demo" : "Log out"} sub={state.demo ? "Return to the welcome screen" : "Unlink this computer and remove its message history"} onClick={() => openModal("confirmLogout")} />
      </div>
      <p class="hint" style={{ padding: "4px 6px" }}>
        Ripple is an independent, open-source client. It is not affiliated with, endorsed by or supported by WhatsApp or Meta. Using unofficial clients can go against WhatsApp's terms and may put your account at risk.
      </p>
    </>
  );
}

export function Settings() {
  const [tab, setTab] = createSignal<Tab>((state.modal?.props?.tab as Tab) ?? "appearance");
  return (
    <Sheet size="wide" class="settings-sheet" head={undefined}>
      <div class="settings">
        <nav class="settings-nav">
          <h2>Settings</h2>
          <For each={TABS}>
            {(t) => (
              <button class={tab() === t.id ? "on" : ""} onClick={() => setTab(t.id)}>
                <Icon name={t.icon} />{t.label}
              </button>
            )}
          </For>
          <div class="ver">Ripple {state.version}{state.demo ? " · demo" : ""}</div>
        </nav>
        <div class="settings-main">
          <div class="sheet-head">
            <h2>{TABS.find((t) => t.id === tab())?.label}</h2>
            <IconButton icon="x" tip="Close" onClick={() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))} />
          </div>
          <Show when={tab()} keyed>
            {(t) => (
              <div class="settings-pane">
                <Switch>
                  <Match when={t === "profile"}><ProfilePane /></Match>
                  <Match when={t === "appearance"}><AppearancePane /></Match>
                  <Match when={t === "notifications"}><NotificationsPane /></Match>
                  <Match when={t === "privacy"}><PrivacyPane /></Match>
                  <Match when={t === "chats"}><ChatsPane /></Match>
                  <Match when={t === "storage"}><StoragePane /></Match>
                  <Match when={t === "shortcuts"}><ShortcutsPane /></Match>
                  <Match when={t === "account"}><AccountPane /></Match>
                </Switch>
              </div>
            )}
          </Show>
        </div>
      </div>
    </Sheet>
  );
}
