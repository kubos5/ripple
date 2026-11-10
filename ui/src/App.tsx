import { Match, Show, Switch, createEffect, onCleanup, onMount } from "solid-js";
import { inTauri } from "./lib/ipc";
import { act, boot, closeChat, derived, openChat, openModal, setState, state } from "./lib/state";
import { Conversation } from "./components/Conversation";
import { Lightbox } from "./components/Lightbox";
import { ModalHost } from "./components/Modals";
import { SidePanel } from "./components/Panel";
import { Rail, Sidebar } from "./components/Sidebar";
import { Welcome } from "./components/Welcome";
import { CallBanner } from "./components/Call";
import { MenuHost, Toasts, closeMenu } from "./components/ui";

function useAppearance() {
  const media = matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    const s = state.settings;
    const root = document.documentElement;
    const dark = s.theme === "dark" || (s.theme === "system" && media.matches);
    root.dataset.theme = dark ? "dark" : "light";
    root.dataset.accent = s.accent;
    root.dataset.density = s.compact ? "compact" : "comfortable";
    root.dataset.motion = s.reduceMotion === "on" ? "reduce" : s.reduceMotion === "off" ? "full" : "system";
    root.dataset.platform = state.platform;
    root.dataset.shell = inTauri ? "tauri" : "browser";
    root.style.setProperty("--scale", String(s.textScale));
    try {
      localStorage.setItem("ripple.theme", s.theme);
      localStorage.setItem("ripple.accent", s.accent);
    } catch { /* storage unavailable */ }
  };
  createEffect(apply);
  media.addEventListener("change", apply);
  onCleanup(() => media.removeEventListener("change", apply));
}

function useShortcuts() {
  const onKey = (e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    const main = state.session.state !== "welcome" && state.session.state !== "pairing" && state.session.state !== "loggedOut";
    if (!main) return;
    if (mod && key === "k") { e.preventDefault(); closeMenu(); return openModal("palette"); }
    if (mod && key === "n") { e.preventDefault(); return openModal(e.shiftKey ? "newGroup" : "newChat"); }
    if (mod && key === ",") { e.preventDefault(); return openModal("settings"); }
    if (mod && key === "f" && !e.shiftKey && state.convo.chat) { e.preventDefault(); return setState("panel", "search"); }
    if (mod && e.shiftKey && key === "f") { e.preventDefault(); setState({ view: "chats" }); return window.dispatchEvent(new Event("ripple:focus-search")); }
    if (mod && key === "i" && e.shiftKey && state.convo.chat) { e.preventDefault(); return setState("panel", state.panel === "info" ? null : "info"); }
    if (mod && e.shiftKey && key === "u" && state.convo.chat) { e.preventDefault(); const id = state.convo.chat; closeChat(); return void act("markUnread", { chat: id }); }
    if (mod && e.shiftKey && key === "d") { e.preventDefault(); return void act("demoTrigger", { what: "call" }); }
    if ((e.altKey || mod) && (e.key === "ArrowDown" || e.key === "ArrowUp") && !e.shiftKey && (e.altKey || !(e.target as HTMLElement)?.closest?.("textarea,input"))) {
      const list = state.view === "archived" ? derived.archived() : derived.inbox();
      if (!list.length) return;
      e.preventDefault();
      const i = list.findIndex((c) => c.id === state.convo.chat);
      const next = list[Math.max(0, Math.min(list.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) void openChat(next.id);
      return;
    }
    if (e.key === "Escape" && !e.defaultPrevented) {
      if (state.modal || state.lightbox) return;
      if (state.selecting) return setState("selecting", false);
      if (state.panel) return setState("panel", null);
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input")) return;
      if (state.convo.chat && !state.reply && !state.editing) closeChat();
    }
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));
}

export function App() {
  onMount(() => void boot());
  useAppearance();
  useShortcuts();
  // The webview's own context menu has nothing useful outside text fields.
  const noMenu = (e: MouseEvent) => {
    if (!(e.target as HTMLElement).closest("input, textarea, .selectable, .rich")) e.preventDefault();
  };
  const showMain = () => ["online", "offline", "connecting"].includes(state.session.state);
  return (
    <div class="app" onContextMenu={noMenu}>
      <Switch>
        <Match when={state.bootError}>
          <div class="blank" style={{ flex: 1 }}>
            <h2 class="display">Ripple couldn't start</h2>
            <p>{state.bootError}</p>
            <button class="btn primary" onClick={() => location.reload()}>Try again</button>
          </div>
        </Match>
        <Match when={!state.ready}>
          <div class="blank" style={{ flex: 1 }}><div class="ripple-mark"><i /><i /><i /><b /></div></div>
        </Match>
        <Match when={!showMain()}>
          <Welcome />
        </Match>
        <Match when={true}>
          <Rail />
          <Sidebar />
          <main class="main">
            <Conversation />
            <Show when={state.panel && state.convo.chat}><SidePanel /></Show>
          </main>
        </Match>
      </Switch>
      <Show when={state.lightbox}><Lightbox /></Show>
      <ModalHost />
      <CallBanner />
      <MenuHost />
      <Toasts />
    </div>
  );
}
