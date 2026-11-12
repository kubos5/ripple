// Shared primitives: icons, avatars, menus, sheets, toasts.

import { For, type JSX, Show, createEffect, createSignal, onCleanup, onMount, splitProps } from "solid-js";
import { Portal } from "solid-js/web";
import { ICONS, type IconName } from "../lib/icons";
import { initials, isGroupJid, tone } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import { avatarOf, chatName, closeModal, state } from "../lib/state";

export type { IconName };

export function Icon(props: { name: IconName; size?: number; class?: string; style?: JSX.CSSProperties }) {
  return (
    <svg
      class={`icon ${props.class ?? ""}`}
      viewBox="0 -960 960 960"
      aria-hidden="true"
      style={{ ...(props.size ? { "font-size": `${props.size}px` } : {}), ...props.style }}
      innerHTML={ICONS[props.name]}
    />
  );
}

export function IconButton(
  props: { icon: IconName; tip?: string; tipPos?: "top" | "right"; on?: boolean; small?: boolean; size?: number } &
    Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, "children">,
) {
  const [own, rest] = splitProps(props, ["icon", "tip", "tipPos", "on", "small", "size", "class"]);
  return (
    <button
      type="button"
      class={`icon-btn ${own.small ? "sm" : ""} ${own.on ? "on" : ""} ${own.class ?? ""}`}
      data-tip={own.tip}
      data-tip-pos={own.tipPos}
      aria-label={own.tip}
      {...rest}
    >
      <Icon name={own.icon} size={own.size} />
    </button>
  );
}

/** Avatar for a chat or person. Falls back to tinted initials. */
export function Avatar(props: { id: string; size?: number; name?: string; src?: string | null; kind?: "person" | "group" | "community"; online?: boolean }) {
  const kind = () => props.kind ?? (isGroupJid(props.id) ? (state.groups[props.id]?.isCommunity ? "community" : "group") : "person");
  const token = () => (props.src !== undefined ? props.src : avatarOf(props.id));
  const label = () => props.name ?? chatName(props.id);
  const [broken, setBroken] = createSignal(false);
  createEffect(() => {
    token();
    setBroken(false);
  });
  return (
    <div class="avatar-wrap">
      <div
        class={`avatar ${kind() === "person" ? "" : kind()}`}
        style={{ "--size": `${props.size ?? 44}px`, "--tone": `var(--tone-${tone(props.id)})` }}
      >
        <Show
          when={token() && !broken()}
          fallback={
            <Show when={kind() === "person"} fallback={<Icon name={kind() === "community" ? "users-three-fill" : "users-fill"} />}>
              <span>{initials(label().replace(/^~/, ""))}</span>
            </Show>
          }
        >
          <img src={mediaUrl(token())} alt="" draggable={false} loading="lazy" onError={() => setBroken(true)} />
        </Show>
      </div>
      <Show when={props.online}><span class="online" /></Show>
    </div>
  );
}

// ───────────────────────────── menus ─────────────────────────────

export interface MenuEntry {
  label?: string;
  icon?: IconName;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  sep?: boolean;
  title?: string;
  hidden?: boolean;
  run?: () => void;
}

interface OpenMenu {
  x: number;
  y: number;
  items: MenuEntry[];
  /** Custom content instead of items (emoji bars and the like). */
  render?: (close: () => void) => JSX.Element;
  align?: "left" | "right";
  class?: string;
  /** The control that opened the menu; clicking it again closes the menu. */
  anchor?: HTMLElement | null;
  /** Custom placement from the menu's measured size (still kept inside the window). */
  place?: (width: number, height: number) => { left: number; top: number };
}

const [menu, setMenu] = createSignal<OpenMenu | null>(null);

export function closeMenu() {
  setMenu(null);
}

export const menuOpen = () => menu() !== null;

/** Opens a context menu at a pointer event or anchored under an element. */
export function openMenu(at: MouseEvent | HTMLElement, items: MenuEntry[], opts: Partial<OpenMenu> = {}) {
  const anchor =
    at instanceof HTMLElement ? at : at.type === "click" && at.currentTarget instanceof HTMLElement ? at.currentTarget : null;
  const open = menu();
  if (anchor && open && open.anchor === anchor) {
    if (!(at instanceof HTMLElement)) at.preventDefault();
    closeMenu();
    return;
  }
  let x: number, y: number;
  if (at instanceof HTMLElement) {
    const r = at.getBoundingClientRect();
    x = opts.align === "right" ? r.right : r.left;
    y = r.bottom + 6;
  } else {
    at.preventDefault();
    x = at.clientX;
    y = at.clientY;
  }
  setMenu({ x, y, items: items.filter((i) => !i.hidden), anchor, ...opts });
}

export function MenuHost() {
  let el: HTMLDivElement | undefined;
  const [pos, setPos] = createSignal({ left: 0, top: 0 });

  createEffect(() => {
    const m = menu();
    if (!m || !el) return;
    // Measure, then keep the menu inside the window.
    const w = el.offsetWidth, h = el.offsetHeight, pad = 8;
    let left = m.align === "right" ? m.x - w : m.x;
    let top = m.y;
    if (m.place) ({ left, top } = m.place(w, h));
    else if (left + w > innerWidth - pad) {
      // Anchored menus slide back into view; pointer menus flip to the other side of the cursor.
      left = m.align ? Math.max(pad, innerWidth - pad - w) : Math.max(pad, m.x - w);
    }
    if (left < pad) left = pad;
    if (!m.place && top + h > innerHeight - pad) top = Math.max(pad, m.y - h - (m.align ? 44 : 0));
    top = Math.max(pad, top);
    setPos({ left, top });
  });

  const onKey = (e: KeyboardEvent) => {
    if (!menu()) return;
    if (e.key === "Escape") { e.stopPropagation(); closeMenu(); return; }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const items = [...(el?.querySelectorAll<HTMLButtonElement>(".menu-item:not(:disabled)") ?? [])];
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    }
  };
  const onDown = (e: MouseEvent) => {
    const m = menu();
    // A press on the control that opened the menu is left to its click, which toggles the menu shut.
    if (m && el && !el.contains(e.target as Node) && !m.anchor?.contains(e.target as Node)) closeMenu();
  };
  onMount(() => {
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("resize", closeMenu);
  });
  onCleanup(() => {
    window.removeEventListener("keydown", onKey, true);
    window.removeEventListener("mousedown", onDown, true);
    window.removeEventListener("blur", closeMenu);
    window.removeEventListener("resize", closeMenu);
  });

  return (
    <Show when={menu()}>
      {(m) => (
        <Portal>
          <div
            ref={el}
            class={`menu ${m().class ?? ""}`}
            role="menu"
            style={{ left: `${pos().left}px`, top: `${pos().top}px` }}
            onContextMenu={(e) => e.preventDefault()}
          >
            <Show when={m().render} fallback={
              <For each={m().items}>
                {(item) => (
                  <Show when={!item.sep} fallback={<div class="menu-sep" />}>
                    <Show when={!item.title} fallback={<div class="menu-title">{item.title}</div>}>
                      <button
                        type="button"
                        role="menuitem"
                        class={`menu-item ${item.danger ? "danger" : ""}`}
                        disabled={item.disabled}
                        onClick={() => {
                          closeMenu();
                          item.run?.();
                        }}
                      >
                        <Show when={item.icon}>{(i) => <Icon name={i()} />}</Show>
                        <span>{item.label}</span>
                        <Show when={item.hint}><span class="hint">{item.hint}</span></Show>
                      </button>
                    </Show>
                  </Show>
                )}
              </For>
            }>
              {m().render!(closeMenu)}
            </Show>
          </div>
        </Portal>
      )}
    </Show>
  );
}

// ───────────────────────────── sheets (modals) ─────────────────────────────

export function Sheet(props: {
  title?: string;
  size?: "wide" | "mid";
  onClose?: () => void;
  children: JSX.Element;
  foot?: JSX.Element;
  head?: JSX.Element;
  flush?: boolean;
  class?: string;
}) {
  const close = () => (props.onClose ?? closeModal)();
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      close();
    }
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));
  return (
    <Portal>
      <div class="scrim" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div class={`sheet ${props.size ?? ""} ${props.class ?? ""}`} role="dialog" aria-modal="true" aria-label={props.title}>
          <Show when={props.title || props.head}>
            <div class="sheet-head">
              <Show when={props.head} fallback={<h2>{props.title}</h2>}>{props.head}</Show>
              <IconButton icon="x" tip="Close" onClick={close} />
            </div>
          </Show>
          <div class={`sheet-body ${props.flush ? "flush" : ""}`}>{props.children}</div>
          <Show when={props.foot}><div class="sheet-foot">{props.foot}</div></Show>
        </div>
      </div>
    </Portal>
  );
}

/** A yes/no confirmation built on `Sheet`. */
export function Confirm(props: { title: string; body: string; action: string; danger?: boolean; onConfirm: () => void; children?: JSX.Element }) {
  return (
    <Sheet
      title={props.title}
      foot={
        <>
          <button class="btn ghost" onClick={closeModal}>Cancel</button>
          <button
            class={`btn ${props.danger ? "danger" : "primary"}`}
            ref={(el) => setTimeout(() => el.focus(), 30)}
            onClick={() => {
              closeModal();
              props.onConfirm();
            }}
          >
            {props.action}
          </button>
        </>
      }
    >
      <p class="muted" style={{ "line-height": 1.5 }}>{props.body}</p>
      {props.children}
    </Sheet>
  );
}

export function Toasts() {
  return (
    <div class="toasts" aria-live="polite">
      <For each={state.toasts}>
        {(t) => (
          <div class={`toast ${t.tone}`}>
            <Show when={t.tone !== "info"}>
              <Icon name={t.tone === "error" ? "warning-circle-fill" : "check-circle-fill"} />
            </Show>
            {t.text}
          </div>
        )}
      </For>
    </div>
  );
}

export function Switch(props: { on: boolean; onChange: (v: boolean) => void; label?: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      class="switch"
      aria-checked={props.on}
      aria-label={props.label}
      disabled={props.disabled}
      onClick={() => props.onChange(!props.on)}
    />
  );
}
