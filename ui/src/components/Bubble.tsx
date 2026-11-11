// One message in the conversation: the bubble, its content, reactions and actions.

import { For, type JSX, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, untrack } from "solid-js";
import { inTauri, mediaUrl } from "../lib/ipc";
import { clock, duration, ephemeralLabel, fileSize, fullStamp, phone, plural, tone } from "../lib/format";
import { type Block, type Inline, jumboEmoji, parse } from "../lib/rich";
import {
  act, contactName, freshIds, isAdmin, jumpToMessage, openModal, previewText, setState, startChat, state, toast, toggleSelect,
} from "../lib/state";
import type { Content, ContentOf, Media, Message, Quote, Reaction } from "../lib/types";
import { KIND_ICON, Ticks, bumpStars } from "./Sidebar";
import { EmojiPicker } from "./Pickers";
import { Avatar, Icon, IconButton, type IconName, type MenuEntry, closeMenu, openMenu } from "./ui";

// ───────────────────────────── rich text ─────────────────────────────

function InlineNodes(props: { nodes: Inline[] }): JSX.Element {
  return (
    <For each={props.nodes}>
      {(n) => {
        switch (n.t) {
          case "text": return <>{n.v}</>;
          case "b": return <strong><InlineNodes nodes={n.c} /></strong>;
          case "i": return <em><InlineNodes nodes={n.c} /></em>;
          case "s": return <s><InlineNodes nodes={n.c} /></s>;
          case "code": return <code>{n.v}</code>;
          case "link": return <a href={n.url} target="_blank" rel="noreferrer noopener" onClick={(e) => e.stopPropagation()}>{n.v}</a>;
          case "mention":
            return (
              <span class="mention" onClick={(e) => { e.stopPropagation(); openModal("contact", { jid: n.jid }); }}>
                @{contactName(n.jid, { you: false }).replace(/^~/, "")}
              </span>
            );
        }
      }}
    </For>
  );
}

export function RichText(props: { text: string; mentions?: string[]; tail?: JSX.Element; class?: string }) {
  const blocks = createMemo<Block[]>(() => parse(props.text, props.mentions ?? []));
  const jumbo = createMemo(() => jumboEmoji(props.text));
  const lastIsPara = () => blocks()[blocks().length - 1]?.t === "p";
  return (
    <div class={`rich ${jumbo() ? `jumbo n${jumbo()}` : ""} ${props.class ?? ""}`} dir="auto">
      <For each={blocks()}>
        {(b, i) => {
          const tail = () => (i() === blocks().length - 1 ? props.tail : undefined);
          switch (b.t) {
            case "p": return <p><InlineNodes nodes={b.c} />{tail()}</p>;
            case "quote": return <blockquote><InlineNodes nodes={b.c} /></blockquote>;
            case "pre": return <pre>{b.v}</pre>;
            case "ul": return <ul><For each={b.items}>{(it) => <li><InlineNodes nodes={it} /></li>}</For></ul>;
            case "ol": return <ol start={b.start}><For each={b.items}>{(it) => <li><InlineNodes nodes={it} /></li>}</For></ol>;
          }
        }}
      </For>
      <Show when={!lastIsPara()}>{props.tail}</Show>
    </div>
  );
}

// ───────────────────────────── meta line ─────────────────────────────

function metaWidth(m: Message) {
  let w = clock(m.ts).length * 6.3 + 10;
  if (m.fromMe) w += 19;
  if (m.edited) w += 40;
  if (m.starred) w += 15;
  if (m.pinnedUntil > Date.now()) w += 15;
  if (m.expires) w += 15;
  return Math.round(w);
}

function Meta(props: { m: Message; mode: "abs" | "row" | "over" | "float" }) {
  const m = () => props.m;
  return (
    <div class={`meta ${props.mode}`} title={fullStamp(m().ts)}>
      <Show when={m().pinnedUntil > state.now}><Icon name="push-pin-fill" /></Show>
      <Show when={m().starred}><Icon name="star-fill" /></Show>
      <Show when={m().expires > 0}><Icon name="timer" /></Show>
      <Show when={m().edited > 0}><span>Edited</span></Show>
      <span>{clock(m().ts)}</span>
      <Show when={m().fromMe} keyed>
        <Show when={m().status} keyed>{(s) => <Ticks status={s} class="tick-swap" />}</Show>
      </Show>
    </div>
  );
}

// ───────────────────────────── audio player (one at a time) ─────────────────────────────

const audio = typeof Audio !== "undefined" ? new Audio() : (null as unknown as HTMLAudioElement);
const [playing, setPlaying] = createSignal<{ id: string; paused: boolean; pos: number; len: number } | null>(null);
const [rate, setRate] = createSignal(1);
if (audio) {
  audio.preload = "auto";
  audio.addEventListener("timeupdate", () => {
    const p = playing();
    if (p) setPlaying({ ...p, pos: audio.currentTime, len: Number.isFinite(audio.duration) ? audio.duration : p.len });
  });
  audio.addEventListener("ended", () => setPlaying(null));
  audio.addEventListener("pause", () => { const p = playing(); if (p && !audio.ended) setPlaying({ ...p, paused: true }); });
  audio.addEventListener("play", () => { const p = playing(); if (p) setPlaying({ ...p, paused: false }); });
  audio.addEventListener("error", () => { if (playing()) { setPlaying(null); toast("This audio can't be played", "error"); } });
}

function togglePlay(m: Message, c: ContentOf<"audio">, startAt?: number) {
  const p = playing();
  if (p?.id === m.id) {
    if (startAt !== undefined) audio.currentTime = startAt;
    if (audio.paused) void audio.play(); else if (startAt === undefined) audio.pause();
    return;
  }
  audio.src = mediaUrl(c.media.token, c.media.mime);
  audio.playbackRate = rate();
  setPlaying({ id: m.id, paused: false, pos: startAt ?? 0, len: c.duration });
  if (startAt) audio.currentTime = startAt;
  void audio.play().catch(() => {});
  if (!m.fromMe && c.voice) void act("played", { chat: m.chat, id: m.id });
}

export function stopAudio() {
  if (audio && playing()) {
    audio.pause();
    setPlaying(null);
  }
}

// ───────────────────────────── media helpers ─────────────────────────────

function wantAuto(c: Content): boolean {
  const s = state.settings;
  switch (c.type) {
    case "image": case "sticker": return s.autoImages;
    case "audio": return s.autoAudio;
    case "video": return c.gif ? s.autoImages : s.autoVideo;
    case "document": return s.autoDocuments;
    default: return false;
  }
}

/** Fetches remote media once when the bubble appears, if the settings say so. */
function useAutoDownload(m: () => Message) {
  createEffect(() => {
    const msg = m();
    const media = "media" in msg.content ? (msg.content.media as Media) : null;
    if (media?.state === "remote" && !msg.viewOnce && untrack(() => wantAuto(msg.content))) {
      void act("download", { chat: msg.chat, id: msg.id });
    }
  });
}

const download = (m: Message) => void act("download", { chat: m.chat, id: m.id });

function fitBox(w: number, h: number, maxW = 330, maxH = 380) {
  const ratio = w > 0 && h > 0 ? w / h : 1;
  let width = Math.min(maxW, Math.max(150, w || maxW));
  let height = width / ratio;
  if (height > maxH) { height = maxH; width = Math.max(150, height * ratio); }
  if (height < 90) height = 90;
  return { width: `${Math.round(width)}px`, "aspect-ratio": `${Math.round(width)} / ${Math.round(height)}` };
}

const thumbUrl = (t: string | null | undefined) => (t ? `data:image/jpeg;base64,${t}` : undefined);

function StatePill(props: { media: Media; size?: boolean; onGet: () => void; play?: boolean }) {
  return (
    <div class="center">
      <Switch>
        <Match when={props.media.state === "downloading"}>
          <span class="pill round"><Icon name="circle-notch" class="ring-progress" /></span>
        </Match>
        <Match when={props.media.state === "remote" || props.media.state === "failed"}>
          <button class="pill" onClick={(e) => { e.stopPropagation(); props.onGet(); }}>
            <Icon name={props.media.state === "failed" ? "arrow-clockwise" : "download-simple"} size={17} />
            {props.media.state === "failed" ? "Retry" : fileSize(props.media.size)}
          </button>
        </Match>
        <Match when={props.play}>
          <span class="pill round"><Icon name="play-fill" /></span>
        </Match>
      </Switch>
    </div>
  );
}

export function Photo(props: { m: Message; c: ContentOf<"image"> | ContentOf<"video">; over?: boolean; tile?: boolean; extra?: number }) {
  const [ready, setReady] = createSignal(false);
  const isVideo = () => props.c.type === "video";
  const gif = () => props.c.type === "video" && props.c.gif;
  const open = () => {
    if (props.c.media.state === "ready") setState("lightbox", { chat: props.m.chat, id: props.m.id });
    else if (props.c.media.state !== "downloading") download(props.m);
  };
  return (
    <div
      class={`photo ${props.over ? "has-over" : ""}`}
      style={props.tile ? undefined : fitBox(props.c.width, props.c.height)}
      data-photo={props.m.id}
      onClick={(e) => { e.stopPropagation(); open(); }}
    >
      <Show when={thumbUrl(props.c.thumb)}>{(t) => <img class="blur" src={t()} alt="" draggable={false} />}</Show>
      <Show when={props.c.media.state === "ready"}>
        <Show
          when={gif()}
          fallback={
            <Show when={!isVideo()} fallback={<img class={`full ${ready() ? "ready" : ""}`} src={thumbUrl(props.c.thumb)} alt="" onLoad={() => setReady(true)} draggable={false} />}>
              <img class={`full ${ready() ? "ready" : ""}`} src={mediaUrl(props.c.media.token)} alt="" loading="lazy" decoding="async" draggable={false} onLoad={() => setReady(true)} />
            </Show>
          }
        >
          <video class={`ready`} src={mediaUrl(props.c.media.token)} autoplay loop muted playsinline />
        </Show>
      </Show>
      <div class="shade" />
      <StatePill media={props.c.media} onGet={() => download(props.m)} play={isVideo() && !gif() && props.c.media.state === "ready"} />
      <Show when={gif()}><span class="tag">GIF</span></Show>
      <Show when={isVideo() && !gif()}>
        <span class="tag"><Icon name="video-camera" />{duration((props.c as ContentOf<"video">).duration)}</span>
      </Show>
      <Show when={props.extra}><div class="more">+{props.extra}</div></Show>
    </div>
  );
}

function ViewOnce(props: { m: Message }) {
  const kind = () => (props.m.content.type === "video" ? "Video" : props.m.content.type === "audio" ? "Voice message" : "Photo");
  const media = () => (props.m.content as ContentOf<"image">).media;
  const openIt = () => {
    if (props.m.fromMe) return;
    if (media().state === "ready") setState("lightbox", { chat: props.m.chat, id: props.m.id });
    else download(props.m);
  };
  return (
    <div class={`once ${props.m.fromMe ? "opened" : ""}`} onClick={(e) => { e.stopPropagation(); openIt(); }}>
      <span class="ring">
        <Show when={media().state === "downloading"} fallback={<Icon name="number-circle-one" size={19} />}>
          <Icon name="circle-notch" size={18} class="ring-progress" />
        </Show>
      </span>
      <span>{kind()}{props.m.fromMe ? "" : media().state === "ready" ? " · tap to view" : ""}</span>
    </div>
  );
}

function Voice(props: { m: Message; c: ContentOf<"audio"> }) {
  const mine = () => (playing()?.id === props.m.id ? playing() : null);
  const frac = () => { const p = mine(); return p && p.len > 0 ? Math.min(1, p.pos / p.len) : 0; };
  const bars = createMemo(() => {
    const w = props.c.waveform;
    if (w.length >= 16) return w.length > 48 ? w.filter((_, i) => i % Math.ceil(w.length / 48) === 0) : w;
    return Array.from({ length: 40 }, (_, i) => 30 + ((i * 37) % 55));
  });
  const ready = () => props.c.media.state === "ready";
  const seek = (e: PointerEvent) => {
    if (!ready()) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    togglePlay(props.m, props.c, f * (mine()?.len || props.c.duration));
  };
  const cycle = (e: MouseEvent) => {
    e.stopPropagation();
    const next = rate() === 1 ? 1.5 : rate() === 1.5 ? 2 : 1;
    setRate(next);
    if (audio) audio.playbackRate = next;
  };
  return (
    <div class="voice" onClick={(e) => e.stopPropagation()}>
      <button
        class="play"
        aria-label={mine() && !mine()!.paused ? "Pause" : "Play"}
        onClick={() => (ready() ? togglePlay(props.m, props.c) : props.c.media.state !== "downloading" && download(props.m))}
      >
        <Switch>
          <Match when={props.c.media.state === "downloading"}><Icon name="circle-notch" class="ring-progress" /></Match>
          <Match when={!ready()}><Icon name={props.c.media.state === "failed" ? "arrow-clockwise" : "download-simple"} /></Match>
          <Match when={mine() && !mine()!.paused}><Icon name="pause-fill" class="swap" /></Match>
          <Match when={true}><Icon name="play-fill" class="swap" /></Match>
        </Switch>
      </button>
      <div class="grow">
        <div class="wave" onPointerDown={seek}>
          <For each={bars()}>
            {(v, i) => <i class={i() / bars().length < frac() ? "done" : ""} style={{ height: `${Math.max(10, v)}%` }} />}
          </For>
        </div>
        <div class="info">
          <span>{duration(mine() ? mine()!.pos : props.c.duration)}</span>
          <Show when={mine()} fallback={<Show when={!props.c.voice}><span class="row" style={{ gap: "4px" }}><Icon name="headphones" size={12} />Audio</span></Show>}>
            <button class="speed" onClick={cycle}>{rate()}×</button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function docIcon(name: string, mime: string): IconName {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (ext === "pdf") return "file-pdf";
  if (["zip", "rar", "7z", "gz", "tar"].includes(ext)) return "file-zip";
  if (["doc", "docx", "rtf", "odt"].includes(ext)) return "file-doc";
  if (["xls", "xlsx", "csv", "ods"].includes(ext)) return "file-xls";
  if (mime.startsWith("image/")) return "file-image";
  if (mime.startsWith("audio/")) return "file-audio";
  if (mime.startsWith("video/")) return "file-video";
  if (["txt", "md", "log", "json"].includes(ext)) return "file-text";
  return "file";
}

export async function saveMedia(m: Message) {
  const media = "media" in m.content ? (m.content.media as Media) : null;
  if (!media?.token) return toast("Download the file first");
  const name = m.content.type === "document" ? m.content.name : `ripple-${m.id.slice(-8)}.${media.token.split(".").pop()}`;
  if (inTauri) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const to = await save({ defaultPath: name });
    if (to && (await act("saveMedia", { chat: m.chat, id: m.id, to })) !== undefined) toast("Saved", "success");
    return;
  }
  const a = document.createElement("a");
  a.href = mediaUrl(media.token);
  a.download = name;
  a.target = "_blank";
  a.click();
}

function Doc(props: { m: Message; c: ContentOf<"document"> }) {
  const ext = () => (props.c.name.includes(".") ? props.c.name.split(".").pop()! : props.c.media.mime.split("/").pop() ?? "file");
  const click = () => {
    const s = props.c.media.state;
    if (s === "ready") void saveMedia(props.m);
    else if (s !== "downloading") download(props.m);
  };
  return (
    <div class="doc" onClick={(e) => { e.stopPropagation(); click(); }}>
      <div class="ficon"><Icon name={docIcon(props.c.name, props.c.media.mime)} /></div>
      <div class="grow">
        <div class="fname">{props.c.name}</div>
        <div class="fmeta">{ext()} · {fileSize(props.c.media.size)}{props.c.pages ? ` · ${plural(props.c.pages, "page")}` : ""}</div>
      </div>
      <span class="icon-btn sm" style={{ "pointer-events": "none" }}>
        <Switch>
          <Match when={props.c.media.state === "downloading"}><Icon name="circle-notch" class="ring-progress" /></Match>
          <Match when={props.c.media.state === "ready"}><Icon name="arrow-square-out" /></Match>
          <Match when={true}><Icon name="download-simple" /></Match>
        </Switch>
      </span>
    </div>
  );
}

function LinkCard(props: { p: NonNullable<ContentOf<"text">["preview"]> }) {
  return (
    <a class="linkcard" href={props.p.url} target="_blank" rel="noreferrer noopener" onClick={(e) => e.stopPropagation()}>
      <Show when={thumbUrl(props.p.thumb)}>{(t) => <img src={t()} alt="" draggable={false} />}</Show>
      <div class="lc-body">
        <div class="lc-title">{props.p.title}</div>
        <Show when={props.p.description}><div class="lc-desc">{props.p.description}</div></Show>
        <div class="lc-site"><Icon name="link-simple" />{props.p.site || new URL(props.p.url).hostname}</div>
      </div>
    </a>
  );
}

function Place(props: { c: ContentOf<"location"> }) {
  const href = () => `https://www.openstreetmap.org/?mlat=${props.c.lat}&mlon=${props.c.lng}#map=16/${props.c.lat}/${props.c.lng}`;
  return (
    <a class="place" href={href()} target="_blank" rel="noreferrer noopener" onClick={(e) => e.stopPropagation()}>
      <div class="map"><span class="pin-dot" /><Icon name="map-pin-fill" class="pin" /></div>
      <div style={{ padding: "7px 8px 3px" }}>
        <div class="p-name">{props.c.name || (props.c.live ? "Live location" : "Pinned location")}</div>
        <Show when={props.c.address}><div class="p-addr">{props.c.address}</div></Show>
        <div class="p-open">{props.c.lat.toFixed(4)}, {props.c.lng.toFixed(4)} · Open map</div>
      </div>
    </a>
  );
}

function Cards(props: { c: ContentOf<"contacts"> }) {
  return (
    <div class="vcard">
      <For each={props.c.cards}>
        {(card) => (
          <div class="v-main">
            <Avatar id={card.jid ?? card.name} size={40} name={card.name} kind="person" src={card.jid ? undefined : null} />
            <div class="grow">
              <div class="v-name truncate">{card.name}</div>
              <div class="v-phone truncate">{card.phone ?? (card.jid ? phone(card.jid) : "Contact")}</div>
            </div>
          </div>
        )}
      </For>
      <Show when={props.c.cards.length === 1}>
        <div class="v-actions">
          <Show when={props.c.cards[0].jid || props.c.cards[0].phone}>
            <button onClick={(e) => {
              e.stopPropagation();
              const card = props.c.cards[0];
              if (card.jid) void startChat(card.jid);
              else openModal("newChat", { phone: card.phone });
            }}>Message</button>
          </Show>
          <button onClick={(e) => { e.stopPropagation(); void navigator.clipboard.writeText(props.c.cards[0].phone ?? props.c.cards[0].name); toast("Copied", "success"); }}>Copy number</button>
        </div>
      </Show>
    </div>
  );
}

function Poll(props: { m: Message; c: ContentOf<"poll"> }) {
  const me = () => state.me?.id ?? "";
  const voters = createMemo(() => new Set(props.c.options.flatMap((o) => o.voters)).size);
  const mine = () => props.c.options.filter((o) => o.voters.includes(me())).map((o) => o.name);
  const vote = (name: string) => {
    let next: string[];
    if (props.c.selectable === 1) next = mine().includes(name) ? [] : [name];
    else next = mine().includes(name) ? mine().filter((n) => n !== name) : [...mine(), name];
    void act("vote", { chat: props.m.chat, id: props.m.id, options: next });
  };
  return (
    <div class={`poll ${props.c.selectable === 1 ? "" : "multi"}`} onClick={(e) => e.stopPropagation()}>
      <div class="p-q">{props.c.question}</div>
      <div class="p-hint"><Icon name={props.c.selectable === 1 ? "check-circle" : "check-square"} />{props.c.selectable === 1 ? "Select one" : "Select one or more"}</div>
      <For each={props.c.options}>
        {(o) => (
          <button class={`opt ${o.voters.includes(me()) ? "mine" : ""}`} onClick={() => vote(o.name)}>
            <div class="line">
              <span class="box"><Icon name="check" /></span>
              <span class="grow">{o.name}</span>
              <span class="faces">
                <For each={o.voters.slice(0, 3)}>{(v) => <Avatar id={v} size={18} name={contactName(v, { you: false })} />}</For>
              </span>
              <span class="n">{o.voters.length}</span>
            </div>
            <div class="bar"><i style={{ width: `${voters() ? (o.voters.length / voters()) * 100 : 0}%` }} /></div>
          </button>
        )}
      </For>
      <div class="p-foot">
        <button onClick={() => openModal("pollVotes", { chat: props.m.chat, id: props.m.id })}>View votes</button>
      </div>
    </div>
  );
}

const monthFmt = new Intl.DateTimeFormat(undefined, { month: "short" });
const eventFmt = new Intl.DateTimeFormat(undefined, { weekday: "long", hour: "numeric", minute: "2-digit" });

function EventCard(props: { m: Message; c: ContentOf<"event"> }) {
  const me = () => state.me?.id ?? "";
  const mine = () => (props.c.responses.going.includes(me()) ? "going" : props.c.responses.maybe.includes(me()) ? "maybe" : props.c.responses.declined.includes(me()) ? "declined" : "");
  const rsvp = (r: string) => void act("rsvp", { chat: props.m.chat, id: props.m.id, response: mine() === r ? "none" : r });
  const going = () => props.c.responses.going;
  return (
    <div class="event" onClick={(e) => e.stopPropagation()}>
      <div class="e-top">
        <div class="e-date"><b>{monthFmt.format(props.c.start)}</b><span>{new Date(props.c.start).getDate()}</span></div>
        <div class="grow">
          <div class={`e-name ${props.c.canceled ? "canceled" : ""}`}>{props.c.name}</div>
          <div class="e-row"><Icon name="clock" />{eventFmt.format(props.c.start)}{props.c.end ? ` – ${clock(props.c.end)}` : ""}</div>
          <Show when={props.c.location}><div class="e-row"><Icon name="map-pin" /><span class="truncate">{props.c.location}</span></div></Show>
        </div>
      </div>
      <Show when={props.c.description}><div class="e-desc">{props.c.description}</div></Show>
      <div class="e-who">
        {going().length ? `${going().slice(0, 3).map((j) => contactName(j, { short: true }).replace(/^~/, "")).join(", ")}${going().length > 3 ? ` +${going().length - 3}` : ""} going` : "No one has responded yet"}
        {props.c.responses.maybe.length ? ` · ${props.c.responses.maybe.length} maybe` : ""}
      </div>
      <Show when={!props.c.canceled}>
        <div class="e-actions">
          <button class={mine() === "going" ? "on" : ""} onClick={() => rsvp("going")}>Going</button>
          <button class={mine() === "maybe" ? "on" : ""} onClick={() => rsvp("maybe")}>Maybe</button>
          <button class={mine() === "declined" ? "on" : ""} onClick={() => rsvp("declined")}>Can't go</button>
        </div>
      </Show>
    </div>
  );
}

// ───────────────────────────── system lines ─────────────────────────────

function names(list: string[], lower = false) {
  const n = list.map((j) => {
    const name = contactName(j).replace(/^~/, "");
    return lower && name === "You" ? "you" : name;
  });
  if (n.length <= 1) return n[0] ?? "someone";
  if (n.length === 2) return `${n[0]} and ${n[1]}`;
  if (n.length <= 4) return `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`;
  return `${n.slice(0, 3).join(", ")} and ${n.length - 3} others`;
}

export function systemText(c: ContentOf<"system">): string {
  const who = c.actor ? contactName(c.actor).replace(/^~/, "") : "Someone";
  const you = who === "You";
  const on = c.text === "on";
  switch (c.event) {
    case "create": return `${who} created the group “${c.text}”`;
    case "add": return `${who} added ${names(c.targets, true)}`;
    case "remove": return `${who} removed ${names(c.targets, true)}`;
    case "leave": return `${who} left`;
    case "join": return `${who} joined using this group's invite link`;
    case "promote": return `${names(c.targets)} ${c.targets.length > 1 || names(c.targets) === "You" ? "are" : "is"} now an admin`;
    case "demote": return `${names(c.targets)} ${c.targets.length > 1 || names(c.targets) === "You" ? "are" : "is"} no longer an admin`;
    case "subject": return `${who} changed the group name to “${c.text}”`;
    case "description": return `${who} changed the group description`;
    case "icon": return `${who} changed this group's picture`;
    case "pin": return `${who} pinned a message`;
    case "ephemeral": {
      const secs = Number(c.text) || 0;
      return secs
        ? `${who} turned on disappearing messages. New messages will disappear from this chat after ${ephemeralLabel(secs)}.`
        : `${who} turned off disappearing messages.`;
    }
    case "announce": return on ? `${who} changed this group's settings so only admins can send messages` : `${who} changed this group's settings so all members can send messages`;
    case "locked": return on ? `${who} changed the settings so only admins can edit group info` : `${who} changed the settings so all members can edit group info`;
    case "approval": return on ? `${who} turned on admin approval to join this group` : `${who} turned off admin approval to join this group`;
    case "membersCanAdd": return on ? `${who} allowed all members to add others` : `${who} allowed only admins to add others`;
    default: return c.text || (you ? "You updated this chat" : `${who} updated this chat`);
  }
}

// ───────────────────────────── menus ─────────────────────────────

function messageText(m: Message): string {
  const c = m.content;
  if (c.type === "text") return c.text;
  if ("caption" in c) return c.caption;
  if (c.type === "poll") return c.question;
  if (c.type === "location") return `${c.name ? c.name + "\n" : ""}https://www.openstreetmap.org/?mlat=${c.lat}&mlon=${c.lng}`;
  if (c.type === "contacts") return c.cards.map((x) => `${x.name}${x.phone ? ` ${x.phone}` : ""}`).join("\n");
  if (c.type === "event") return c.name;
  return "";
}

export const EDIT_WINDOW = 15 * 60_000;

export function react(m: Message, emoji: string) {
  void act("react", { chat: m.chat, id: m.id, emoji });
}

export function openReactBar(at: HTMLElement | MouseEvent, m: Message) {
  const mine = m.reactions.find((r) => r.sender === state.me?.id)?.emoji;
  // The action buttons sit on the far side of the bubble, so open back over it.
  const align = m.fromMe ? "left" : "right";
  // Remember where the bar opened so the full picker can replace it in place.
  const anchor = at instanceof HTMLElement ? at : ({ clientX: at.clientX, clientY: at.clientY, preventDefault() {} } as MouseEvent);
  openMenu(at, [], {
    class: "bar",
    align,
    render: (close) => (
      <div class="react-bar">
        <For each={state.settings.quickReactions}>
          {(e, i) => (
            <button class={mine === e ? "mine" : ""} style={{ "--i": i() }} onClick={() => { react(m, e); close(); }}>{e}</button>
          )}
        </For>
        <button
          class="more"
          style={{ "--i": 6 }}
          aria-label="More emoji"
          onClick={() => openMenu(anchor, [], { class: "flush", align, render: (done) => <EmojiPicker onPick={(e) => { react(m, e); done(); }} autofocus /> })}
        >
          <Icon name="plus" />
        </button>
      </div>
    ),
  });
}

export function messageMenu(m: Message): MenuEntry[] {
  const c = m.content;
  const gone = c.type === "deleted";
  const sys = c.type === "system" || c.type === "call";
  const text = messageText(m);
  const hasMedia = "media" in c;
  const canEdit = m.fromMe && (c.type === "text" || (hasMedia && "caption" in c)) && Date.now() - m.ts < EDIT_WINDOW;
  const chat = state.chats[m.chat];
  const pinned = m.pinnedUntil > Date.now();
  const group = m.chat.endsWith("@g.us");
  const canPin = !group || !state.groups[m.chat]?.locked || isAdmin(m.chat);
  return [
    { label: "Reply", icon: "arrow-bend-up-left", hidden: gone || sys || !!chat?.readOnly, run: () => { setState({ reply: m, editing: null }); window.dispatchEvent(new Event("ripple:focus-composer")); } },
    { label: "Reply privately", icon: "user", hidden: gone || sys || !group || m.fromMe, run: () => void startChat(m.sender) },
    { label: "Forward", icon: "share-fat", hidden: gone || sys || m.viewOnce, run: () => openModal("forward", { chat: m.chat, ids: [m.id] }) },
    { label: "Copy", icon: "copy", hidden: !text || gone, run: () => { void navigator.clipboard.writeText(text); toast("Copied", "success"); } },
    { label: m.starred ? "Unstar" : "Star", icon: "star", hidden: gone || sys, run: async () => { await act("star", { chat: m.chat, ids: [m.id], on: !m.starred }); bumpStars((n) => n + 1); } },
    { label: pinned ? "Unpin" : "Pin", icon: "push-pin", hidden: gone || sys || !canPin, run: () => (pinned ? void act("pinMessage", { chat: m.chat, id: m.id, secs: 0 }) : openModal("pin", { chat: m.chat, id: m.id })) },
    { label: "Edit", icon: "pencil-simple", hidden: !canEdit, run: () => { setState({ editing: m, reply: null }); window.dispatchEvent(new Event("ripple:focus-composer")); } },
    { label: "Save as…", icon: "download-simple", hidden: !hasMedia || m.viewOnce || (c as { media: Media }).media.state !== "ready", run: () => void saveMedia(m) },
    {
      label: "Add to sticker favourites", icon: "heart", hidden: c.type !== "sticker" || !(c as ContentOf<"sticker">).media.token,
      run: async () => { await act("stickerFavorite", { token: (c as ContentOf<"sticker">).media.token, on: true }); toast("Added to favourites", "success"); },
    },
    { label: "Message info", icon: "info", hidden: !m.fromMe || gone || sys, run: () => openModal("messageInfo", { chat: m.chat, id: m.id }) },
    { sep: true, hidden: sys },
    { label: "Select", icon: "check-circle", hidden: sys, run: () => toggleSelect(m.id) },
    { label: "Delete", icon: "trash", danger: true, run: () => openModal("delete", { chat: m.chat, ids: [m.id] }) },
  ];
}

// ───────────────────────────── reactions ─────────────────────────────

function Reactions(props: { m: Message }) {
  const groups = createMemo(() => {
    const map = new Map<string, Reaction[]>();
    for (const r of props.m.reactions) map.set(r.emoji, [...(map.get(r.emoji) ?? []), r]);
    return [...map.entries()].sort((a, b) => b[1].length - a[1].length);
  });
  const me = () => state.me?.id;
  return (
    <div class="reactions">
      <For each={groups().slice(0, 4)}>
        {([emoji, list]) => (
          <button
            class={`reaction ${list.some((r) => r.sender === me()) ? "mine" : ""}`}
            title={names(list.map((r) => r.sender))}
            onClick={(e) => {
              e.stopPropagation();
              if (props.m.reactions.length > 1 || !list.some((r) => r.sender === me())) openModal("reactions", { chat: props.m.chat, id: props.m.id });
              else react(props.m, emoji);
            }}
          >
            <span>{emoji}</span>
            <Show when={list.length > 1}><b>{list.length}</b></Show>
          </button>
        )}
      </For>
      <Show when={groups().length > 4}>
        <button class="reaction" onClick={(e) => { e.stopPropagation(); openModal("reactions", { chat: props.m.chat, id: props.m.id }); }}>
          <b>+{groups().length - 4}</b>
        </button>
      </Show>
    </div>
  );
}

function QuoteBox(props: { q: Quote }) {
  return (
    <button class="quote" style={{ "--tone": `var(--tone-${tone(props.q.sender)})` }} onClick={(e) => { e.stopPropagation(); void jumpToMessage(props.q.id); }}>
      <div class="grow">
        <div class="q-name truncate">{contactName(props.q.sender).replace(/^~/, "")}</div>
        <div class="q-text">
          <Show when={KIND_ICON[props.q.kind]}>{(i) => <Icon name={i()} />}</Show>
          {previewText(props.q.text)}
        </div>
      </div>
      <Show when={thumbUrl(props.q.thumb)}>{(t) => <img src={t()} alt="" draggable={false} />}</Show>
    </button>
  );
}

// ───────────────────────────── the row ─────────────────────────────

export interface RowFlags {
  first: boolean;
  last: boolean;
}

export function MessageRow(props: { m: Message; flags: () => RowFlags; album?: () => Message[] | undefined }) {
  const m = () => props.m;
  const c = () => m().content;
  const is = <T extends Content["type"]>(t: T) => (c().type === t ? (c() as ContentOf<T>) : undefined);
  const group = () => m().chat.endsWith("@g.us");
  const [menuOpen, setMenuOpen] = createSignal(false);
  const fresh = freshIds.has(props.m.id);
  useAutoDownload(m);

  const album = () => props.album?.();
  const caption = () => {
    const x = c();
    return "caption" in x ? x.caption : "";
  };
  const visual = () => !m().viewOnce && (c().type === "image" || c().type === "video");
  const bare = () => c().type === "sticker";
  const selected = () => !!state.selected[m().id];
  const showSender = () => group() && !m().fromMe && props.flags().first;
  const textTail = () => <span class="meta-gap" style={{ width: `${metaWidth(m())}px` }} />;

  const menu = (e: MouseEvent) => {
    if (state.selecting) return;
    setMenuOpen(true);
    openMenu(e, messageMenu(m()));
    const off = () => { setMenuOpen(false); window.removeEventListener("mousedown", off, true); };
    setTimeout(() => window.addEventListener("mousedown", off, true), 0);
  };
  onCleanup(closeMenu);

  return (
    <Switch>
      <Match when={is("system")}>
        {(s) => <div class="sys"><span>{systemText(s())}</span></div>}
      </Match>
      <Match when={is("call")}>
        {(call) => (
          <div class={`sys ${call().missed ? "missed" : ""}`}>
            <span>
              <Icon name={call().missed ? "phone-x-fill" : "phone-fill"} />
              {call().missed ? "Missed" : m().fromMe ? "Outgoing" : "Incoming"} {call().video ? "video" : "voice"} call at {clock(m().ts)}
            </span>
          </div>
        )}
      </Match>
      <Match when={true}>
        <div
          class={`msg ${m().fromMe ? "out" : "in"} ${props.flags().first ? "first" : ""} ${props.flags().last ? "last" : ""} ${m().reactions.length ? "has-reactions" : ""} ${selected() ? "selected" : ""} ${state.selecting ? "selecting" : ""} ${state.convo.flash === m().id ? "flash" : ""} ${fresh ? "fresh" : ""} ${menuOpen() ? "menu-open" : ""}`}
          data-mid={m().id}
          onContextMenu={menu}
          onClick={() => state.selecting && toggleSelect(m().id)}
          onDblClick={() => { if (!state.selecting && c().type !== "deleted" && !state.chats[m().chat]?.readOnly) { setState({ reply: m(), editing: null }); window.dispatchEvent(new Event("ripple:focus-composer")); } }}
        >
          <Show when={state.selecting}><span class="pick"><Icon name="check" /></span></Show>
          <Show when={group() && !m().fromMe}>
            <div class="gutter">
              <Show when={props.flags().last}>
                <button onClick={(e) => { e.stopPropagation(); openModal("contact", { jid: m().sender }); }}><Avatar id={m().sender} size={30} /></button>
              </Show>
            </div>
          </Show>
          <div class={`bubble ${bare() ? "bare" : ""} ${visual() || album() ? "media" : ""}`}>
            <Show when={showSender() && !bare()}>
              <div class="sender" style={{ "--tone": `var(--tone-${tone(m().sender)})` }} onClick={(e) => { e.stopPropagation(); openModal("contact", { jid: m().sender }); }}>
                {contactName(m().sender)}
              </div>
            </Show>
            <Show when={m().forwarded > 0 && c().type !== "deleted"}>
              <div class="fwd"><Icon name="share-fat" />{m().forwarded >= 5 ? "Forwarded many times" : "Forwarded"}</div>
            </Show>
            <Show when={m().quote}>{(q) => <QuoteBox q={q()} />}</Show>
            <Switch>
              <Match when={c().type === "deleted"}>
                <div class="gone"><Icon name="prohibit" />{m().fromMe ? "You deleted this message" : "This message was deleted"}{textTail()}</div>
                <Meta m={m()} mode="abs" />
              </Match>
              <Match when={m().viewOnce}>
                <ViewOnce m={m()} />
                <Meta m={m()} mode="row" />
              </Match>
              <Match when={album()}>
                {(list) => (
                  <>
                    <div class={`album n${Math.min(list().length, 4)}`}>
                      <For each={list().slice(0, 4)}>
                        {(item, i) => (
                          <Photo m={item} c={item.content as ContentOf<"image">} tile extra={i() === 3 && list().length > 4 ? list().length - 4 : undefined} />
                        )}
                      </For>
                    </div>
                    <Show when={list().find((x) => "caption" in x.content && x.content.caption)} fallback={<Meta m={m()} mode="over" />}>
                      {(cap) => (
                        <div class="body-pad" style={{ position: "relative" }}>
                          <RichText text={(cap().content as ContentOf<"image">).caption} mentions={cap().mentions} tail={textTail()} />
                          <Meta m={m()} mode="abs" />
                        </div>
                      )}
                    </Show>
                  </>
                )}
              </Match>
              <Match when={is("text")}>
                {(t) => (
                  <>
                    <Show when={t().preview}>{(p) => <LinkCard p={p()} />}</Show>
                    <RichText text={t().text} mentions={m().mentions} tail={textTail()} />
                    <Meta m={m()} mode="abs" />
                  </>
                )}
              </Match>
              <Match when={is("image") ?? is("video")}>
                {(media) => (
                  <>
                    <Photo m={m()} c={media()} over={!caption()} />
                    <Show when={caption()} fallback={<Meta m={m()} mode="over" />}>
                      <div class="body-pad" style={{ position: "relative", "max-width": fitBox(media().width, media().height).width }}>
                        <RichText text={caption()} mentions={m().mentions} tail={textTail()} />
                        <Meta m={m()} mode="abs" />
                      </div>
                    </Show>
                  </>
                )}
              </Match>
              <Match when={is("audio")}>
                {(a) => (<><Voice m={m()} c={a()} /><Meta m={m()} mode="row" /></>)}
              </Match>
              <Match when={is("document")}>
                {(d) => (
                  <>
                    <Doc m={m()} c={d()} />
                    <Show when={d().caption} fallback={<Meta m={m()} mode="row" />}>
                      <div style={{ position: "relative", "margin-top": "5px", "max-width": "320px" }}>
                        <RichText text={d().caption} mentions={m().mentions} tail={textTail()} />
                        <Meta m={m()} mode="abs" />
                      </div>
                    </Show>
                  </>
                )}
              </Match>
              <Match when={is("sticker")}>
                {(s) => (
                  <div style={{ position: "relative", "padding-bottom": "16px" }}>
                    <Show when={s().media.state === "ready"} fallback={<div class="sticker-skel"><Icon name={s().media.state === "downloading" ? "circle-notch" : "sticker"} class={s().media.state === "downloading" ? "ring-progress" : ""} size={26} /></div>}>
                      <img class="sticker" src={mediaUrl(s().media.token)} alt="Sticker" draggable={false} loading="lazy" />
                    </Show>
                    <Meta m={m()} mode="float" />
                  </div>
                )}
              </Match>
              <Match when={is("location")}>
                {(l) => (<><Place c={l()} /><Meta m={m()} mode="row" /></>)}
              </Match>
              <Match when={is("contacts")}>
                {(v) => (<><Cards c={v()} /><Show when={v().cards.length !== 1}><Meta m={m()} mode="row" /></Show></>)}
              </Match>
              <Match when={is("poll")}>
                {(p) => <Poll m={m()} c={p()} />}
              </Match>
              <Match when={is("event")}>
                {(e) => (<><EventCard m={m()} c={e()} /><Meta m={m()} mode="row" /></>)}
              </Match>
              <Match when={true}>
                <div class="gone"><Icon name="info" />This message type isn't supported yet{textTail()}</div>
                <Meta m={m()} mode="abs" />
              </Match>
            </Switch>
            <Show when={m().reactions.length}><Reactions m={m()} /></Show>
          </div>
          <Show when={c().type !== "deleted"}>
            <div class="msg-actions">
              <IconButton icon="smiley" tip="React" tipPos="top" onClick={(e) => { e.stopPropagation(); openReactBar(e.currentTarget, m()); }} />
              <Show when={!state.chats[m().chat]?.readOnly}>
                <IconButton icon="arrow-bend-up-left" tip="Reply" tipPos="top" onClick={(e) => { e.stopPropagation(); setState({ reply: m(), editing: null }); window.dispatchEvent(new Event("ripple:focus-composer")); }} />
              </Show>
              <IconButton icon="dots-three" tip="More" tipPos="top" onClick={(e) => { e.stopPropagation(); menu(e); }} />
            </div>
          </Show>
        </div>
      </Match>
    </Switch>
  );
}
