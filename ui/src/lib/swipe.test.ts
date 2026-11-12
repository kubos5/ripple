import { describe, expect, it } from "vitest";
import { SWIPE_AT, SwipeTracker, rubber } from "./swipe";

/** Feeds a list of [dx, dy] events 16ms apart and returns the steps. */
function run(t: SwipeTracker, events: [number, number][], from = 0) {
  return events.map(([dx, dy], i) => t.feed(dx, dy, from + i * 16));
}

describe("swipe to reply", () => {
  it("follows a rightward swipe and ends when events stop", () => {
    const t = new SwipeTracker();
    const steps = run(t, [[2, 0], [6, 1], [10, 0], [14, 1], [12, 0], [15, 0], [11, 0]]);
    expect(steps[0]).toBe("pass");
    expect(steps[1]).toBe("start");
    expect(steps.slice(2).every((s) => s === "move")).toBe(true);
    expect(t.raw).toBeGreaterThanOrEqual(SWIPE_AT);
    expect(t.gap()).toBe("end");
    expect(t.phase).toBe("idle");
  });

  it("leaves vertical scrolling alone for the whole gesture", () => {
    const t = new SwipeTracker();
    const steps = run(t, [[0, 5], [1, 8], [20, 2], [30, 0]]);
    expect(steps.every((s) => s === "pass")).toBe(true);
    expect(t.gap()).toBeNull();
  });

  it("ignores leftward swipes", () => {
    const t = new SwipeTracker();
    expect(run(t, [[-8, 0], [-12, 0]])).toEqual(["pass", "pass"]);
  });

  it("treats steadily fading events as the fingers leaving the trackpad", () => {
    const t = new SwipeTracker();
    run(t, [[6, 0], [12, 0], [14, 0], [16, 0], [15, 0], [16, 0]]);
    const fade = run(t, [[13, 0], [11, 0], [9, 0], [7, 0], [5, 0], [4, 0]], 200);
    expect(fade).toContain("end");
    // Whatever inertia remains is swallowed, not scrolled.
    expect(t.feed(3, 0, 400)).toBe("swallow");
    expect(t.gap()).toBeNull();
  });

  it("can be pulled back under the threshold before letting go", () => {
    const t = new SwipeTracker();
    run(t, [[10, 0], [30, 0], [40, 0], [-30, 0], [-25, 0]]);
    expect(t.raw).toBeLessThan(SWIPE_AT);
  });

  it("stiffens past the threshold", () => {
    expect(rubber(-5)).toBe(0);
    expect(rubber(SWIPE_AT)).toBe(SWIPE_AT);
    expect(rubber(SWIPE_AT + 20)).toBeLessThan(SWIPE_AT + 20);
    expect(rubber(1000)).toBe(SWIPE_AT + 36);
  });
});
