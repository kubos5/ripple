// Emoji and sticker pickers.

import { For, Show, createMemo, createResource, createSignal, onMount } from "solid-js";
import { call, mediaUrl } from "../lib/ipc";
import { act, send, toast } from "../lib/state";
import type { Attachment, Sticker } from "../lib/types";
import { Icon, type IconName, openMenu } from "./ui";

type Row = [emoji: string, name: string, tags: string, group: number, skins?: string[]];

let cache: Promise<Row[]> | null = null;
const loadEmoji = () => (cache ??= import("../assets/emoji.json").then((m) => m.default as unknown as Row[]));

const GROUPS: { id: number; label: string; icon: IconName }[] = [
  { id: 0, label: "Smileys & emotion", icon: "smiley" },
  { id: 1, label: "People", icon: "hand-waving" },
  { id: 3, label: "Animals & nature", icon: "sparkle" },
  { id: 4, label: "Food & drink", icon: "heart" },
  { id: 5, label: "Travel & places", icon: "globe" },
  { id: 6, label: "Activities", icon: "lightning" },
  { id: 7, label: "Objects", icon: "bell" },
  { id: 8, label: "Symbols", icon: "hash" },
  { id: 9, label: "Flags", icon: "flag" },
];

const RECENT_KEY = "ripple.emoji.recent";
const SKIN_KEY = "ripple.emoji.skin";
const readRecent = (): string[] => {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]"); } catch { return []; }
};
const [recent, setRecent] = createSignal<string[]>(readRecent());
const [skin, setSkin] = createSignal<number>(Number(localStorage.getItem(SKIN_KEY) ?? 0));

export function noteEmojiUse(e: string) {
  const next = [e, ...recent().filter((x) => x !== e)].slice(0, 32);
  setRecent(next);
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(next)); } catch { /* storage unavailable */ }
}

export function EmojiPicker(props: { onPick: (emoji: string) => void; autofocus?: boolean }) {
  const [rows] = createResource(loadEmoji);
  const [query, setQuery] = createSignal("");
  const [hover, setHover] = createSignal<Row | null>(null);
  let scroller: HTMLDivElement | undefined;
  let input: HTMLInputElement | undefined;
  onMount(() => props.autofocus && setTimeout(() => input?.focus(), 40));

  const tone = (r: Row) => (skin() > 0 && r[4] ? r[4][skin() - 1] : r[0]);
  const found = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return null;
    const words = q.split(/\s+/);
    return (rows() ?? []).filter((r) => words.every((w) => r[1].includes(w) || r[2].includes(w))).slice(0, 120);
  });
  const byGroup = createMemo(() => {
    const map = new Map<number, Row[]>();
    for (const r of rows() ?? []) (map.get(r[3]) ?? map.set(r[3], []).get(r[3])!).push(r);
    return map;
  });
  const pick = (e: string) => {
    noteEmojiUse(e);
    props.onPick(e);
  };
  const Cell = (p: { r: Row }) => (
    <button class="emoji" title={p.r[1]} onMouseEnter={() => setHover(p.r)} onClick={() => pick(tone(p.r))}>{tone(p.r)}</button>
  );
  const skinMenu = (e: MouseEvent) => {
    e.stopPropagation();
    const variants = ["✋", "✋🏻", "✋🏼", "✋🏽", "✋🏾", "✋🏿"];
    const row = e.currentTarget as HTMLElement;
    const next = (skin() + 1) % variants.length;
    setSkin(next);
    try { localStorage.setItem(SKIN_KEY, String(next)); } catch { /* storage unavailable */ }
    row.animate([{ transform: "scale(0.7)" }, { transform: "scale(1)" }], { duration: 220, easing: "cubic-bezier(0.22, 1, 0.36, 1)" });
  };
  return (
    <div class="emoji-picker" onMouseLeave={() => setHover(null)}>
      <div class="ep-search">
        <Icon name="magnifying-glass" />
        <input ref={input} placeholder="Search emoji" value={query()} spellcheck={false} onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && found()?.[0]) pick(tone(found()![0])); }} />
      </div>
      <Show when={!found()}>
        <div class="ep-tabs">
          <Show when={recent().length}>
            <button aria-label="Recent" onClick={() => scroller?.querySelector("[data-g='r']")?.scrollIntoView({ block: "start" })}><Icon name="clock" /></button>
          </Show>
          <For each={GROUPS}>
            {(g) => (
              <button aria-label={g.label} title={g.label} onClick={() => scroller?.querySelector(`[data-g='${g.id}']`)?.scrollIntoView({ block: "start" })}>
                <Icon name={g.icon} />
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="ep-scroll" ref={scroller}>
        <Show when={rows()} fallback={<div class="empty"><div class="spinner" /></div>}>
          <Show when={found()} fallback={
            <>
              <Show when={recent().length}>
                <div class="ep-title" data-g="r">Recent</div>
                <div class="ep-grid">
                  <For each={recent()}>{(e) => <button class="emoji" onClick={() => pick(e)}>{e}</button>}</For>
                </div>
              </Show>
              <For each={GROUPS}>
                {(g) => (
                  <section>
                    <div class="ep-title" data-g={g.id}>{g.label}</div>
                    <div class="ep-grid">
                      <For each={byGroup().get(g.id) ?? []}>{(r) => <Cell r={r} />}</For>
                    </div>
                  </section>
                )}
              </For>
            </>
          }>
            {(list) => (
              <Show when={list().length} fallback={<div class="empty">No emoji found</div>}>
                <div class="ep-grid" style={{ "padding-top": "8px" }}><For each={list()}>{(r) => <Cell r={r} />}</For></div>
              </Show>
            )}
          </Show>
        </Show>
      </div>
      <div class="ep-foot">
        <span class="truncate grow">{hover() ? `${tone(hover()!)}  ${hover()![1]}` : "Pick an emoji"}</span>
        <button class="skin" title="Skin tone" onClick={skinMenu}>{["✋", "✋🏻", "✋🏼", "✋🏽", "✋🏾", "✋🏿"][skin()]}</button>
      </div>
    </div>
  );
}

// ───────────────────────────── stickers ─────────────────────────────

export function fileToAttachment(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Could not read that file"));
    reader.onload = () => resolve({ data: reader.result as string, name: file.name, mime: file.type });
    reader.readAsDataURL(file);
  });
}

export function pickFiles(accept: string, multiple = true): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.multiple = multiple;
    input.onchange = () => resolve([...(input.files ?? [])]);
    input.addEventListener("cancel", () => resolve([]));
    input.click();
  });
}

export function StickerPicker(props: { onDone: () => void }) {
  const [list, { mutate, refetch }] = createResource(() => call<Sticker[]>("stickers").catch(() => []));
  const [tab, setTab] = createSignal<"recent" | "fav">("recent");
  const shown = () => (list() ?? []).filter((s) => tab() === "recent" || s.favorite);
  const create = async () => {
    const [file] = await pickFiles("image/png,image/jpeg,image/webp", false);
    if (!file) return;
    const res = await act<{ stickers: Sticker[] }>("stickerCreate", { file: await fileToAttachment(file) });
    if (res) {
      mutate(res.stickers);
      toast("Sticker added", "success");
    }
  };
  const menu = (e: MouseEvent, s: Sticker) =>
    openMenu(e, [
      { label: s.favorite ? "Remove from favourites" : "Add to favourites", icon: "heart", run: async () => mutate((await act<Sticker[]>("stickerFavorite", { token: s.token, on: !s.favorite })) ?? list()) },
      { label: "Remove sticker", icon: "trash", danger: true, run: async () => mutate((await act<Sticker[]>("stickerRemove", { token: s.token })) ?? list()) },
    ]);
  return (
    <div class="emoji-picker">
      <div class="ep-tabs wide">
        <button class={tab() === "recent" ? "on" : ""} onClick={() => setTab("recent")}><Icon name="clock" />Recent</button>
        <button class={tab() === "fav" ? "on" : ""} onClick={() => setTab("fav")}><Icon name="heart" />Favourites</button>
        <span class="grow" />
        <button onClick={create} title="Create a sticker from an image"><Icon name="plus" />Create</button>
      </div>
      <div class="ep-scroll">
        <Show when={shown().length} fallback={
          <div class="empty">
            <Icon name="sticker" />
            <strong>{tab() === "fav" ? "No favourite stickers" : "No stickers yet"}</strong>
            {tab() === "fav" ? "Right-click a sticker to add it here." : "Create one from any image, or save stickers people send you."}
          </div>
        }>
          <div class="sticker-grid">
            <For each={shown()}>
              {(s) => (
                <button
                  onContextMenu={(e) => menu(e, s)}
                  onClick={() => { void send({ type: "sticker", token: s.token }).then(() => void refetch()); props.onDone(); }}
                >
                  <img src={mediaUrl(s.token)} alt="Sticker" loading="lazy" draggable={false} />
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}
