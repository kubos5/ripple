// Full-window viewer for photos and videos.

import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { mediaUrl } from "../lib/ipc";
import { fullStamp } from "../lib/format";
import { act, contactName, openModal, setState, state } from "../lib/state";
import type { ContentOf, Message } from "../lib/types";
import { RichText, saveMedia } from "./Bubble";
import { bumpStars } from "./Sidebar";
import { IconButton } from "./ui";

const isViewable = (m: Message) => (m.content.type === "image" || m.content.type === "video") && m.content.media.state === "ready";

export function Lightbox() {
  const [closing, setClosing] = createSignal(false);
  const [zoom, setZoom] = createSignal(1);
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  const [dragging, setDragging] = createSignal(false);
  // The message that opened the viewer may live outside the loaded page (gallery); keep it reachable.
  const items = createMemo(() => state.convo.messages.filter((m) => isViewable(m) && (!m.viewOnce || m.id === state.lightbox?.id)));
  const current = () => items().find((m) => m.id === state.lightbox?.id);
  const index = () => items().findIndex((m) => m.id === state.lightbox?.id);
  const media = () => current()?.content as ContentOf<"image"> | ContentOf<"video"> | undefined;

  const close = () => {
    if (closing()) return;
    setClosing(true);
    setTimeout(() => setState("lightbox", null), 110);
  };
  const step = (d: number) => {
    const next = items()[index() + d];
    if (next && state.lightbox) setState("lightbox", { chat: state.lightbox.chat, id: next.id });
  };
  createEffect(on(() => state.lightbox?.id, () => { setZoom(1); setPan({ x: 0, y: 0 }); }));
  createEffect(() => { if (state.lightbox && !current() && !closing()) close(); });

  const onKey = (e: KeyboardEvent) => {
    if (state.modal) return;
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === "ArrowLeft") step(-1);
    else if (e.key === "ArrowRight") step(1);
    else if (e.key === "+" || e.key === "=") setZoom((z) => Math.min(4, z + 0.5));
    else if (e.key === "-") { setZoom((z) => Math.max(1, z - 0.5)); if (zoom() === 1) setPan({ x: 0, y: 0 }); }
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));

  const onWheel = (e: WheelEvent) => {
    if (media()?.type !== "image") return;
    e.preventDefault();
    const next = Math.min(4, Math.max(1, zoom() * (e.deltaY < 0 ? 1.12 : 0.89)));
    setZoom(next);
    if (next === 1) setPan({ x: 0, y: 0 });
  };
  const onDown = (e: PointerEvent) => {
    if (zoom() === 1) return;
    e.preventDefault();
    const start = { x: e.clientX - pan().x, y: e.clientY - pan().y };
    let moved = false;
    setDragging(true);
    const move = (ev: PointerEvent) => { moved = true; setPan({ x: ev.clientX - start.x, y: ev.clientY - start.y }); };
    const up = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (!moved) { setZoom(1); setPan({ x: 0, y: 0 }); }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <Show when={current()}>
      {(m) => (
        <div class={`lightbox ${closing() ? "closing" : ""}`} onContextMenu={(e) => e.preventDefault()}>
          <div class="lb-top drag">
            <div class="grow">
              <div class="name truncate">{contactName(m().sender)}</div>
              <div class="when">{fullStamp(m().ts)}</div>
            </div>
            <Show when={!m().viewOnce}>
              <IconButton class="no-drag" icon={m().starred ? "star-fill" : "star"} tip={m().starred ? "Unstar" : "Star"} onClick={async () => { await act("star", { chat: m().chat, ids: [m().id], on: !m().starred }); bumpStars((n) => n + 1); }} />
              <IconButton class="no-drag" icon="share-fat" tip="Forward" onClick={() => openModal("forward", { chat: m().chat, ids: [m().id] })} />
              <IconButton class="no-drag" icon="download-simple" tip="Save" onClick={() => void saveMedia(m())} />
            </Show>
            <Show when={media()?.type === "image"}>
              <IconButton class="no-drag" icon={zoom() > 1 ? "magnifying-glass-minus" : "magnifying-glass-plus"} tip={zoom() > 1 ? "Zoom out" : "Zoom in"} onClick={() => { setZoom(zoom() > 1 ? 1 : 2); setPan({ x: 0, y: 0 }); }} />
            </Show>
            <IconButton class="no-drag" icon="x" tip="Close" onClick={close} />
          </div>
          <div class="lb-stage" onMouseDown={(e) => e.target === e.currentTarget && close()} onWheel={onWheel}>
            <Show when={index() > 0}><button class="lb-nav prev" aria-label="Previous" onClick={() => step(-1)}><svg class="icon" viewBox="0 0 256 256"><path d="M165.7 202.3a8 8 0 0 1-11.4 11.4l-80-80a8 8 0 0 1 0-11.4l80-80a8 8 0 0 1 11.4 11.4L91.3 128Z" /></svg></button></Show>
            <Show when={index() < items().length - 1}><button class="lb-nav next" aria-label="Next" onClick={() => step(1)}><svg class="icon" viewBox="0 0 256 256"><path d="m181.7 133.7-80 80a8 8 0 0 1-11.4-11.4L164.7 128 90.3 53.7a8 8 0 0 1 11.4-11.4l80 80a8 8 0 0 1 0 11.4Z" /></svg></button></Show>
            <Show when={media()} keyed>
              {(c) => (
                <Show when={c.type === "image"} fallback={<video src={mediaUrl(c.media.token)} controls autoplay loop={c.type === "video" && c.gif} />}>
                  <img
                    src={mediaUrl(c.media.token)}
                    alt=""
                    draggable={false}
                    class={`${zoom() > 1 ? "zoomed" : ""} ${dragging() ? "zooming" : ""}`}
                    style={{ transform: `translate(${pan().x}px, ${pan().y}px) scale(${zoom()})`, animation: "pop-in var(--t-slow) var(--spring)" }}
                    onPointerDown={onDown}
                    onClick={() => { if (zoom() === 1) setZoom(2); }}
                  />
                </Show>
              )}
            </Show>
          </div>
          <Show when={media()?.caption}>
            <div class="lb-caption"><RichText text={media()!.caption} mentions={m().mentions} /></div>
          </Show>
          <Show when={items().length > 1 && !m().viewOnce}>
            <div class="lb-strip">
              <For each={items().slice(Math.max(0, index() - 6), index() + 7)}>
                {(it) => (
                  <button class={it.id === m().id ? "on" : ""} onClick={() => setState("lightbox", { chat: it.chat, id: it.id })}>
                    <Show when={(it.content as ContentOf<"image">).thumb}>{(t) => <img src={`data:image/jpeg;base64,${t()}`} alt="" />}</Show>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
}
