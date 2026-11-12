// Full-window viewer for photos and videos.

import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { mediaUrl } from "../lib/ipc";
import { fullStamp } from "../lib/format";
import { act, contactName, openModal, setState, state } from "../lib/state";
import type { ContentOf, Message } from "../lib/types";
import { RichText, saveMedia } from "./Bubble";
import { bumpStars } from "./Sidebar";
import { Icon, IconButton } from "./ui";

const isViewable = (m: Message) => (m.content.type === "image" || m.content.type === "video") && m.content.media.state === "ready";

export function Lightbox() {
  const [zoom, setZoom] = createSignal(1);
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  const [dragging, setDragging] = createSignal(false);
  // The message that opened the viewer may live outside the loaded page (gallery); keep it reachable.
  const items = createMemo(() => state.convo.messages.filter((m) => isViewable(m) && (!m.viewOnce || m.id === state.lightbox?.id)));
  const current = () => items().find((m) => m.id === state.lightbox?.id);
  const index = () => items().findIndex((m) => m.id === state.lightbox?.id);
  const media = () => current()?.content as ContentOf<"image"> | ContentOf<"video"> | undefined;

  const close = () => setState("lightbox", null);
  const step = (d: number) => {
    const next = items()[index() + d];
    if (next && state.lightbox) setState("lightbox", { chat: state.lightbox.chat, id: next.id });
  };
  createEffect(on(() => state.lightbox?.id, () => { setZoom(1); setPan({ x: 0, y: 0 }); }));
  createEffect(() => { if (state.lightbox && !current()) close(); });

  // ── zoom and pan ──
  // The photo is centred in the stage and scaled about its centre, so `pan` is the
  // offset of its centre from the stage's. Panning stops once an edge reaches the stage.
  let stage: HTMLDivElement | undefined;
  let img: HTMLImageElement | undefined;
  const clampPan = (p: { x: number; y: number }, z = zoom()) => {
    if (!img || !stage) return { x: 0, y: 0 };
    const bx = Math.max(0, (img.offsetWidth * z - stage.clientWidth) / 2);
    const by = Math.max(0, (img.offsetHeight * z - stage.clientHeight) / 2);
    return { x: Math.min(bx, Math.max(-bx, p.x)), y: Math.min(by, Math.max(-by, p.y)) };
  };
  /** Zooms to `z`, keeping the point under `at` (a client position) where it is. */
  const zoomTo = (z: number, at?: { x: number; y: number }) => {
    const next = Math.min(4, Math.max(1, z));
    if (next === 1) {
      setZoom(1);
      return setPan({ x: 0, y: 0 });
    }
    let p = pan();
    if (at && stage) {
      const r = stage.getBoundingClientRect();
      const q = { x: at.x - (r.left + r.width / 2), y: at.y - (r.top + r.height / 2) };
      const k = next / zoom();
      p = { x: q.x - (q.x - p.x) * k, y: q.y - (q.y - p.y) * k };
    }
    setZoom(next);
    setPan(clampPan(p, next));
  };

  const onKey = (e: KeyboardEvent) => {
    if (state.modal) return;
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === "ArrowLeft") step(-1);
    else if (e.key === "ArrowRight") step(1);
    else if (e.key === "+" || e.key === "=") zoomTo(zoom() + 0.5);
    else if (e.key === "-") zoomTo(zoom() - 0.5);
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));

  // Scrolling never zooms. Zoomed out it does nothing; zoomed in it moves the photo,
  // in both directions, so a trackpad pans it like a page. Pinching zooms: Chromium
  // reports a pinch as a wheel event with ctrlKey (as Ctrl + wheel does), WebKit as
  // gesture events.
  let pinching = false;
  let pinchFrom = 1;
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    if (media()?.type !== "image" || pinching) return;
    if (e.ctrlKey) {
      const d = Math.max(-25, Math.min(25, e.deltaY));
      return zoomTo(zoom() * Math.exp(-d * 0.01), { x: e.clientX, y: e.clientY });
    }
    if (zoom() === 1) return;
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? (stage?.clientHeight ?? 400) : 1;
    setPan(clampPan({ x: pan().x - e.deltaX * unit, y: pan().y - e.deltaY * unit }));
  };
  type Gesture = UIEvent & { scale: number; clientX: number; clientY: number };
  const onGesture = (e: Event) => {
    e.preventDefault();
    if (media()?.type !== "image") return;
    const g = e as Gesture;
    if (e.type === "gesturestart") { pinching = true; pinchFrom = zoom(); }
    else if (e.type === "gestureend") pinching = false;
    else zoomTo(pinchFrom * g.scale, { x: g.clientX, y: g.clientY });
  };
  const bindStage = (el: HTMLDivElement) => {
    stage = el;
    el.addEventListener("wheel", onWheel, { passive: false });
    for (const t of ["gesturestart", "gesturechange", "gestureend"]) el.addEventListener(t, onGesture);
  };

  const onDown = (e: PointerEvent) => {
    if (zoom() === 1) return;
    e.preventDefault();
    const start = { x: e.clientX - pan().x, y: e.clientY - pan().y };
    let moved = false;
    setDragging(true);
    const move = (ev: PointerEvent) => { moved = true; setPan(clampPan({ x: ev.clientX - start.x, y: ev.clientY - start.y })); };
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
        <div class="lightbox" onContextMenu={(e) => e.preventDefault()}>
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
              <IconButton class="no-drag" icon={zoom() > 1 ? "magnifying-glass-minus" : "magnifying-glass-plus"} tip={zoom() > 1 ? "Zoom out" : "Zoom in"} onClick={() => zoomTo(zoom() > 1 ? 1 : 2)} />
            </Show>
            <IconButton class="no-drag" icon="x" tip="Close" onClick={close} />
          </div>
          <div class="lb-stage" ref={bindStage}>
            <Show when={index() > 0}><button class="lb-nav prev" aria-label="Previous" onClick={() => step(-1)}><Icon name="caret-left" /></button></Show>
            <Show when={index() < items().length - 1}><button class="lb-nav next" aria-label="Next" onClick={() => step(1)}><Icon name="caret-right" /></button></Show>
            {/* The media is fitted inside this box (never cropped); clicking around it closes. */}
            <div class="lb-fit" onMouseDown={(e) => e.target === e.currentTarget && close()}>
              <Show when={media()} keyed>
                {(c) => (
                  <Show when={c.type === "image"} fallback={<video src={mediaUrl(c.media.token)} controls autoplay loop={c.type === "video" && c.gif} />}>
                    <img
                      ref={(el) => (img = el)}
                      src={mediaUrl(c.media.token)}
                      alt=""
                      draggable={false}
                      class={`${zoom() > 1 ? "zoomed" : ""} ${dragging() ? "dragging" : ""}`}
                      style={{ transform: `translate(${pan().x}px, ${pan().y}px) scale(${zoom()})` }}
                      onPointerDown={onDown}
                      onClick={(e) => { if (zoom() === 1) zoomTo(2, { x: e.clientX, y: e.clientY }); }}
                    />
                  </Show>
                )}
              </Show>
            </div>
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
