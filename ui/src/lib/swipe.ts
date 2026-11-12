// Two-finger trackpad swipe to the right on a message to reply to it.
//
// Trackpads report two-finger movement as wheel events. The DOM gives no "fingers
// lifted" signal, so a swipe ends when the events stop, or when they start fading
// away smoothly, which is the inertia macOS adds after the fingers leave the pad.

/** How far (px) a message has to be pulled before letting go replies to it. */
export const SWIPE_AT = 64;

/** Where the message is drawn for a pull of `raw` px: 1:1 up to the threshold, then stiffer. */
export function rubber(raw: number) {
  if (raw <= SWIPE_AT) return Math.max(0, raw);
  return Math.min(SWIPE_AT + 36, SWIPE_AT + (raw - SWIPE_AT) * 0.35);
}

export type SwipeStep = "pass" | "start" | "move" | "end" | "swallow";

/**
 * Follows one wheel gesture at a time. `feed` takes horizontal finger movement
 * (positive = rightwards) and vertical scroll; `gap` is called once events stop.
 */
export class SwipeTracker {
  phase: "idle" | "deciding" | "swipe" | "scroll" | "done" = "idle";
  /** Rightward pull so far, in px. */
  raw = 0;
  private sx = 0;
  private sy = 0;
  private began = 0;
  private prev = 0;
  private fading = 0;

  feed(dx: number, dy: number, t: number): SwipeStep {
    if (this.phase === "idle") {
      this.phase = "deciding";
      this.sx = this.sy = 0;
      this.began = t;
    }
    switch (this.phase) {
      case "deciding": {
        this.sx += dx;
        this.sy += dy;
        if (Math.abs(this.sx) + Math.abs(this.sy) < 4) return "pass";
        // Clearly sideways and to the right; anything else is a scroll until the gesture ends.
        if (this.sx > 0 && Math.abs(this.sx) > Math.abs(this.sy) * 1.5) {
          this.phase = "swipe";
          this.raw = this.sx;
          this.prev = Math.abs(dx);
          this.fading = 0;
          return "start";
        }
        this.phase = "scroll";
        return "pass";
      }
      case "scroll":
        return "pass";
      case "swipe": {
        this.raw = Math.max(0, this.raw + dx);
        const mag = Math.abs(dx);
        // Inertia: several events in a row, each a bit weaker than the last.
        this.fading = t - this.began > 80 && mag > 0 && mag < this.prev * 0.92 ? this.fading + 1 : 0;
        this.prev = mag;
        if (this.fading >= 4) {
          this.phase = "done";
          return "end";
        }
        return "move";
      }
      case "done":
        return "swallow";
    }
  }

  /** No events for a while: the gesture is over. Returns "end" if a swipe was in progress. */
  gap(): "end" | null {
    const was = this.phase;
    this.phase = "idle";
    return was === "swipe" ? "end" : null;
  }

  /** Treats the rest of this gesture as an ordinary scroll. */
  cancel() {
    this.phase = "scroll";
  }
}

const MAC = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

/**
 * Rightward finger movement for a wheel event. Content follows the fingers with natural
 * scrolling, so the pull is the opposite of the scroll delta. WebKit on macOS says which
 * way the user's scrolling goes, so there the pull always matches the fingers.
 */
function fingerX(e: WheelEvent) {
  const inverted = (e as WheelEvent & { webkitDirectionInvertedFromDevice?: boolean }).webkitDirectionInvertedFromDevice;
  return MAC && typeof inverted === "boolean" && !inverted ? e.deltaX : -e.deltaX;
}

/**
 * Wires swipe-to-reply onto a message list. Rows are `.msg[data-mid]` elements holding a
 * `.bubble`; `hint` is a fixed-position element showing the reply arrow.
 */
export function swipeToReply(opts: {
  hint: HTMLElement;
  canReply: (id: string) => boolean;
  reply: (id: string) => void;
}) {
  const { hint } = opts;
  const tracker = new SwipeTracker();
  let row: HTMLElement | null = null;
  let bubble: HTMLElement | null = null;
  let id = "";
  let origin = { left: 0, mid: 0 };
  let timer: ReturnType<typeof setTimeout> | undefined;
  let settle: ReturnType<typeof setTimeout> | undefined;

  const draw = () => {
    if (!bubble) return;
    const x = rubber(tracker.raw);
    const p = Math.min(1, x / SWIPE_AT);
    bubble.style.transform = `translateX(${x}px)`;
    // The arrow sits in the space the bubble uncovers, growing as the pull nears the threshold.
    hint.style.opacity = String(p);
    hint.style.transform = `translate(${origin.left + x / 2 - 15}px, ${origin.mid - 15}px) scale(${0.55 + 0.45 * p})`;
    hint.classList.toggle("ready", tracker.raw >= SWIPE_AT);
  };

  const release = () => {
    if (!bubble || !row) return;
    const fire = tracker.raw >= SWIPE_AT;
    const b = bubble;
    clearTimeout(settle);
    row.classList.remove("swiping");
    b.classList.add("swipe-back");
    hint.classList.add("swipe-back");
    b.style.transform = "";
    hint.style.opacity = "0";
    settle = setTimeout(() => {
      b.classList.remove("swipe-back");
      hint.classList.remove("swipe-back", "ready");
    }, 220);
    row = bubble = null;
    if (fire) opts.reply(id);
  };

  return (e: WheelEvent) => {
    if (e.ctrlKey || e.deltaMode !== 0) return;
    clearTimeout(timer);
    timer = setTimeout(() => { if (tracker.gap() === "end") release(); }, 90);
    const step = tracker.feed(fingerX(e), e.deltaY, e.timeStamp);
    if (step === "start") {
      // A mouse's Shift + wheel arrives in big steps; only a trackpad's small ones swipe.
      const r = Math.abs(e.deltaX) < 60 ? (e.target as Element | null)?.closest?.<HTMLElement>(".msg[data-mid]") : null;
      const b = r?.querySelector<HTMLElement>(":scope > .bubble");
      if (!r || !b || !opts.canReply(r.dataset.mid!)) return tracker.cancel();
      e.preventDefault();
      row = r;
      bubble = b;
      id = r.dataset.mid!;
      const box = b.getBoundingClientRect();
      origin = { left: box.left, mid: box.top + box.height / 2 };
      clearTimeout(settle);
      b.classList.remove("swipe-back");
      hint.classList.remove("swipe-back");
      r.classList.add("swiping");
      draw();
    } else if (step === "move") {
      e.preventDefault();
      draw();
    } else if (step === "end") {
      e.preventDefault();
      release();
    } else if (step === "swallow") {
      e.preventDefault();
    }
  };
}
