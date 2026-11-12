// The message composer at the bottom of a conversation.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { call } from "../lib/ipc";
import { duration, tone, userPart } from "../lib/format";
import { firstUrl } from "../lib/rich";
import { act, contactName, noteDraft, openModal, previewText, send, setState, state, toast } from "../lib/state";
import type { Chat, LinkPreview, Message } from "../lib/types";
import { EDIT_WINDOW } from "./Bubble";
import { EmojiPicker, StickerPicker, pickFiles } from "./Pickers";
import { KIND_ICON } from "./Sidebar";
import { Avatar, Icon, IconButton, menuOpen, openMenu } from "./ui";

const MAC = /Mac/.test(navigator.platform);

// The emoji/sticker panel reopens on whichever side was picked last.
type PickerMode = "emoji" | "sticker";
const MODE_KEY = "ripple.picker";
const lastMode = (): PickerMode => {
  try { return localStorage.getItem(MODE_KEY) === "sticker" ? "sticker" : "emoji"; } catch { return "emoji"; }
};

function quoteLabel(m: Message) {
  const c = m.content;
  if (c.type === "text") return previewText(c.text);
  if ("caption" in c && c.caption) return previewText(c.caption);
  if (c.type === "poll") return c.question;
  if (c.type === "event") return c.name;
  if (c.type === "document") return c.name;
  return { image: "Photo", video: "Video", audio: "Voice message", sticker: "Sticker", location: "Location", contacts: "Contact" }[c.type as string] ?? "Message";
}

export function Composer(props: { chat: Chat }) {
  let area!: HTMLTextAreaElement;
  let root!: HTMLDivElement;
  const [text, setText] = createSignal(props.chat.draft ?? "");
  const [picker, setPicker] = createSignal<null | PickerMode>(null);
  const choose = (mode: PickerMode) => {
    setPicker(mode);
    try { localStorage.setItem(MODE_KEY, mode); } catch { /* storage unavailable */ }
  };
  const [suggest, setSuggest] = createSignal<{ query: string; at: number } | null>(null);
  const [active, setActive] = createSignal(0);
  const [preview, setPreview] = createSignal<LinkPreview | null>(null);
  const mentions = new Map<string, string>();
  let dismissedUrl = "";
  let lookedUp = "";

  const chatId = () => props.chat.id;
  const isGroup = () => props.chat.kind !== "direct";
  const blocked = () => !isGroup() && !!state.contacts[chatId()]?.blocked;
  const hasText = () => text().trim().length > 0;

  const resize = () => {
    area.style.height = "auto";
    area.style.height = `${Math.min(180, area.scrollHeight)}px`;
  };
  const focus = () => area?.focus({ preventScroll: true });

  // Typing while nothing else takes text goes to the message box. Focusing it during
  // keydown makes the webview deliver that same key there, so accents, Option/AltGr
  // characters and undo all behave as if the box had been focused already.
  const onType = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.isComposing || !area?.isConnected || document.activeElement === area) return;
    if (e.metaKey || (e.ctrlKey && !e.getModifierState("AltGraph")) || (e.altKey && !MAC && !e.getModifierState("AltGraph"))) return;
    if (e.key.length !== 1 && e.key !== "Dead") return;
    if (state.modal || state.lightbox || state.recording || picker() || menuOpen()) return;
    const t = document.activeElement as HTMLElement | null;
    if (t?.closest("input, textarea, select, [contenteditable]")) return;
    if (e.key === " " && t?.closest("button, a, [role=button], [tabindex]")) return;
    focus();
  };

  const put = (value: string) => {
    setText(value);
    area.value = value;
    resize();
  };

  onMount(() => {
    resize();
    focus();
    const onFocusReq = () => focus();
    const onDown = (e: MouseEvent) => { if (picker() && !root.contains(e.target as Node)) setPicker(null); };
    window.addEventListener("ripple:focus-composer", onFocusReq);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onType);
    onCleanup(() => {
      window.removeEventListener("ripple:focus-composer", onFocusReq);
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onType);
      stopTyping();
      if (state.recording) void act("recordStop", { chat: null, replyTo: null });
      setState("recording", null);
    });
  });

  // Entering edit mode loads the message text; leaving it restores an empty box.
  createEffect(on(() => state.editing, (m, prev) => {
    if (m) {
      const c = m.content;
      put(c.type === "text" ? c.text : "caption" in c ? c.caption : "");
      focus();
      area.setSelectionRange(area.value.length, area.value.length);
    } else if (prev) put("");
  }, { defer: true }));
  createEffect(on(() => state.reply, (m) => m && focus(), { defer: true }));

  // ── typing notifications ──
  let typingSent = 0;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const stopTyping = () => {
    clearTimeout(idleTimer);
    if (typingSent) {
      typingSent = 0;
      void call("typing", { chat: chatId(), state: "idle" }).catch(() => {});
    }
  };
  const noteTyping = () => {
    if (Date.now() - typingSent > 4000) {
      typingSent = Date.now();
      void call("typing", { chat: chatId(), state: "typing" }).catch(() => {});
    }
    clearTimeout(idleTimer);
    idleTimer = setTimeout(stopTyping, 5000);
  };

  // ── link previews ──
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  const checkLink = (value: string) => {
    clearTimeout(previewTimer);
    const url = state.settings.linkPreviews && !state.editing ? firstUrl(value) : null;
    if (!url) {
      lookedUp = "";
      return setPreview(null);
    }
    if (url === lookedUp || url === dismissedUrl) return;
    previewTimer = setTimeout(async () => {
      lookedUp = url;
      const p = await call<LinkPreview | null>("linkPreview", { url }).catch(() => null);
      if (firstUrl(text()) === url) setPreview(p);
    }, 650);
  };

  // ── mentions ──
  const candidates = createMemo(() => {
    const s = suggest();
    if (!s || !isGroup()) return [];
    const q = s.query.toLowerCase();
    const members = state.groups[chatId()]?.participants ?? [];
    return members
      .filter((p) => p.id !== state.me?.id)
      .map((p) => ({ jid: p.id, name: contactName(p.id, { you: false }).replace(/^~/, "") }))
      .filter((p) => p.name.toLowerCase().includes(q) || userPart(p.jid).includes(q))
      .slice(0, 8);
  });
  const detectMention = () => {
    if (!isGroup()) return setSuggest(null);
    const before = area.value.slice(0, area.selectionStart);
    const m = /(^|\s)@([^\s@]{0,24})$/.exec(before);
    setSuggest(m ? { query: m[2], at: before.length - m[2].length - 1 } : null);
    setActive(0);
  };
  const applyMention = (c: { jid: string; name: string }) => {
    const s = suggest();
    if (!s) return;
    const end = area.selectionStart;
    const next = `${area.value.slice(0, s.at)}@${c.name} ${area.value.slice(end)}`;
    mentions.set(c.name, c.jid);
    put(next);
    const caret = s.at + c.name.length + 2;
    area.setSelectionRange(caret, caret);
    setSuggest(null);
    focus();
  };
  /** Swaps "@Display Name" for WhatsApp's "@<number>" form and lists who was mentioned. */
  const resolveMentions = (value: string) => {
    const jids: string[] = [];
    let out = value;
    for (const [name, jid] of [...mentions.entries()].sort((a, b) => b[0].length - a[0].length)) {
      if (out.includes(`@${name}`)) {
        out = out.split(`@${name}`).join(`@${userPart(jid)}`);
        jids.push(jid);
      }
    }
    return { text: out, jids };
  };

  // ── input ──
  const onInput = () => {
    setText(area.value);
    resize();
    detectMention();
    if (!state.editing) {
      noteDraft(chatId(), area.value);
      if (area.value.trim()) noteTyping(); else stopTyping();
    }
    checkLink(area.value);
  };

  const submit = async () => {
    const raw = text().trim();
    if (!raw) return;
    const editing = state.editing;
    const { text: body, jids } = resolveMentions(raw);
    put("");
    mentions.clear();
    stopTyping();
    setPicker(null);
    const p = preview();
    setPreview(null);
    lookedUp = dismissedUrl = "";
    if (editing) {
      setState("editing", null);
      const old = editing.content.type === "text" ? editing.content.text : "caption" in editing.content ? editing.content.caption : "";
      if (body !== old) await act("edit", { chat: editing.chat, id: editing.id, text: body });
      return;
    }
    noteDraft(chatId(), "");
    await send({ type: "text", text: body, preview: p && body.includes(p.url) ? p : null }, { mentions: jids });
  };

  const wrap = (mark: string) => {
    const { selectionStart: a, selectionEnd: b, value } = area;
    if (a === b) return;
    put(`${value.slice(0, a)}${mark}${value.slice(a, b)}${mark}${value.slice(b)}`);
    area.setSelectionRange(a + 1, b + 1);
  };

  const onKey = (e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;
    if (suggest() && candidates().length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = candidates().length;
        return setActive((i) => (i + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        return applyMention(candidates()[active()]);
      }
      if (e.key === "Escape") {
        e.preventDefault();
        return setSuggest(null);
      }
    }
    if (e.key === "Enter" && !e.isComposing) {
      const sends = mod || (state.settings.enterSends && !e.shiftKey && !e.altKey);
      if (sends) {
        e.preventDefault();
        void submit();
      }
      return;
    }
    if (e.key === "Escape") {
      if (picker()) { e.preventDefault(); return setPicker(null); }
      if (state.editing || state.reply) { e.preventDefault(); return void setState({ editing: null, reply: null }); }
      return;
    }
    if (e.key === "ArrowUp" && !area.value && !state.editing) {
      const mine = [...state.convo.messages].reverse().find((m) => m.fromMe && m.content.type === "text" && Date.now() - m.ts < EDIT_WINDOW);
      if (mine) {
        e.preventDefault();
        setState({ editing: mine, reply: null });
      }
      return;
    }
    if (mod && !e.shiftKey && e.key.toLowerCase() === "b") { e.preventDefault(); wrap("*"); }
    if (mod && !e.shiftKey && e.key.toLowerCase() === "i") { e.preventDefault(); wrap("_"); }
    if (mod && e.shiftKey && e.key.toLowerCase() === "x") { e.preventDefault(); wrap("~"); }
  };

  const onPaste = (e: ClipboardEvent) => {
    const files = [...(e.clipboardData?.files ?? [])];
    if (files.length) {
      e.preventDefault();
      openModal("attach", { files });
    }
  };

  const insertEmoji = (emoji: string) => {
    const a = area.selectionStart ?? area.value.length, b = area.selectionEnd ?? a;
    put(area.value.slice(0, a) + emoji + area.value.slice(b));
    const caret = a + emoji.length;
    area.setSelectionRange(caret, caret);
    noteDraft(chatId(), area.value);
    focus();
  };

  const attach = (e: MouseEvent) => {
    setPicker(null);
    openMenu(e.currentTarget as HTMLElement, [
      { label: "Photos & videos", icon: "images", run: async () => { const files = await pickFiles("image/*,video/*"); if (files.length) openModal("attach", { files }); } },
      { label: "Document", icon: "file-text", run: async () => { const files = await pickFiles("*/*"); if (files.length) openModal("attach", { files, asDocument: true }); } },
      { label: "Audio", icon: "headphones", run: async () => { const files = await pickFiles("audio/*"); if (files.length) openModal("attach", { files }); } },
      { sep: true },
      { label: "Poll", icon: "chart-bar", run: () => openModal("poll") },
      { label: "Event", icon: "calendar-blank", run: () => openModal("event") },
      { label: "Contact", icon: "user", run: () => openModal("shareContact") },
      { label: "Location", icon: "map-pin", run: () => openModal("location") },
    ], {
      // Above the message box, where the emoji picker opens, rather than over it.
      place: (_w, h) => {
        const r = root.getBoundingClientRect();
        return { left: r.left + 16, top: r.top + 2 - h };
      },
    });
  };

  // ── voice recording ──
  const startRec = async () => {
    setPicker(null);
    setState("recording", { elapsed: 0, levels: [] });
    try {
      await call("recordStart");
    } catch (e) {
      setState("recording", null);
      toast((e as Error).message, "error");
    }
  };
  const stopRec = async (sendIt: boolean) => {
    const replyTo = sendIt && state.reply ? state.reply.id : null;
    setState("recording", null);
    if (sendIt) setState("reply", null);
    await act("recordStop", { chat: sendIt ? chatId() : null, replyTo });
  };

  const ctx = () => state.editing ?? state.reply;

  return (
    <div class="composer" ref={root}>
      <Show when={picker()}>
        <div class="picker-pop">
          <div class="pp-tabs">
            <button class={picker() === "emoji" ? "on" : ""} onClick={() => choose("emoji")}>Emoji</button>
            <button class={picker() === "sticker" ? "on" : ""} onClick={() => choose("sticker")}>Stickers</button>
          </div>
          <Show when={picker() === "emoji"} fallback={<StickerPicker onDone={() => setPicker(null)} />}>
            <EmojiPicker onPick={insertEmoji} autofocus />
          </Show>
        </div>
      </Show>
      <Show when={suggest() && candidates().length}>
        <div class="suggest">
          <For each={candidates()}>
            {(c, i) => (
              <button class={i() === active() ? "active" : ""} onMouseEnter={() => setActive(i())} onMouseDown={(e) => { e.preventDefault(); applyMention(c); }}>
                <Avatar id={c.jid} size={28} />
                <span class="s-name">{c.name}</span>
                <span class="s-sub">{state.contacts[c.jid]?.pushName && state.contacts[c.jid]?.name ? `~${state.contacts[c.jid]!.pushName}` : ""}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Switch>
        <Match when={blocked()}>
          <div class="composer-note">
            <Icon name="prohibit" />You blocked this contact.
            <button class="btn sm" onClick={() => void act("block", { jid: chatId(), on: false })}>Unblock</button>
          </div>
        </Match>
        <Match when={props.chat.readOnly}>
          <div class="composer-note">
            <Icon name={state.groups[chatId()]?.joined === false ? "sign-out" : "megaphone"} />
            {state.groups[chatId()]?.joined === false ? "You can't send messages because you're no longer in this group." : "Only admins can send messages here."}
          </div>
        </Match>
        <Match when={state.recording}>
          {(rec) => (
            <div class="composer-box">
              <div class="rec">
                <IconButton icon="trash" tip="Discard" tipPos="top" onClick={() => void stopRec(false)} />
                <span class="dot" />
                <span class="time">{duration(rec().elapsed / 1000)}</span>
                <div class="live">
                  <For each={rec().levels.slice(-90)}>{(l) => <i style={{ height: `${Math.max(10, Math.min(100, l * 1.6))}%` }} />}</For>
                </div>
                <button class="send-btn ready" aria-label="Send voice message" onClick={() => void stopRec(true)}>
                  <Icon name="paper-plane-right-fill" />
                </button>
              </div>
            </div>
          )}
        </Match>
        <Match when={true}>
          <div class="composer-box">
            <Show when={ctx()} keyed>
              {(m) => (
                <div class="composer-ctx" style={{ "--tone": state.editing ? "var(--accent)" : `var(--tone-${tone(m.sender)})` }}>
                  <div class="grow">
                    <div class="c-label">
                      <Icon name={state.editing ? "pencil-simple" : "arrow-bend-up-left"} />
                      {state.editing ? "Edit message" : `Replying to ${contactName(m.sender).replace(/^~/, "").replace(/^You$/, "yourself")}`}
                    </div>
                    <div class="c-text truncate">
                      <Show when={KIND_ICON[m.content.type]}>{(i) => <Icon name={i()} size={13} style={{ display: "inline", "vertical-align": "-2px", "margin-right": "4px" }} />}</Show>
                      {quoteLabel(m)}
                    </div>
                  </div>
                  <Show when={"thumb" in m.content && m.content.thumb}>{(t) => <img src={`data:image/jpeg;base64,${t()}`} alt="" />}</Show>
                  <IconButton icon="x" small tip="Cancel" tipPos="top" onClick={() => setState({ editing: null, reply: null })} />
                </div>
              )}
            </Show>
            <Show when={preview()}>
              {(p) => (
                <div class="composer-ctx link">
                  <Show when={p().thumb}>{(t) => <img src={`data:image/jpeg;base64,${t()}`} alt="" />}</Show>
                  <div class="grow">
                    <div class="c-label" style={{ color: "var(--text)" }}><span class="truncate">{p().title}</span></div>
                    <div class="c-text truncate">{p().description || p().site}</div>
                  </div>
                  <IconButton icon="x" small tip="Remove preview" tipPos="top" onClick={() => { dismissedUrl = p().url; setPreview(null); }} />
                </div>
              )}
            </Show>
            <div class="composer-row">
              <IconButton icon="plus" tip="Attach" tipPos="top" onClick={attach} disabled={!!state.editing} />
              <IconButton icon="smiley" tip={picker() ? undefined : "Emoji & stickers"} aria-label="Emoji & stickers" tipPos="top" on={!!picker()} onClick={() => setPicker(picker() ? null : lastMode())} />
              <textarea
                ref={area}
                rows={1}
                placeholder={state.editing ? "Edit message" : "Message"}
                value={text()}
                spellcheck={state.settings.spellcheck}
                onInput={onInput}
                onKeyDown={onKey}
                onPaste={onPaste}
                onClick={detectMention}
                onBlur={() => setTimeout(() => setSuggest(null), 120)}
              />
              <button
                class={`send-btn ${hasText() ? "ready" : ""}`}
                aria-label={hasText() ? "Send" : "Record voice message"}
                data-tip={hasText() ? undefined : "Voice message"}
                data-tip-pos="top"
                onClick={() => (hasText() ? void submit() : state.editing ? setState("editing", null) : void startRec())}
              >
                <Icon name={state.editing ? "check" : "paper-plane-right-fill"} class={hasText() ? "" : "hide"} />
                <Icon name="microphone" class={hasText() || state.editing ? "hide" : ""} />
                <Show when={state.editing && !hasText()}><Icon name="x" /></Show>
              </button>
            </div>
          </div>
        </Match>
      </Switch>
    </div>
  );
}
