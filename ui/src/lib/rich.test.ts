import { describe, expect, it } from "vitest";
import { firstUrl, jumboEmoji, parse, parseInline, plain } from "./rich";

describe("inline formatting", () => {
  it("parses bold, italic, strike and code", () => {
    expect(parseInline("a *b* _c_ ~d~ `e`")).toEqual([
      { t: "text", v: "a " },
      { t: "b", c: [{ t: "text", v: "b" }] },
      { t: "text", v: " " },
      { t: "i", c: [{ t: "text", v: "c" }] },
      { t: "text", v: " " },
      { t: "s", c: [{ t: "text", v: "d" }] },
      { t: "text", v: " " },
      { t: "code", v: "e" },
    ]);
  });

  it("nests marks", () => {
    expect(parseInline("*bold _both_*")).toEqual([
      { t: "b", c: [{ t: "text", v: "bold " }, { t: "i", c: [{ t: "text", v: "both" }] }] },
    ]);
  });

  it("ignores markers inside words and unbalanced markers", () => {
    expect(parseInline("snake_case_name")).toEqual([{ t: "text", v: "snake_case_name" }]);
    expect(parseInline("2 * 3 * 4")).toEqual([{ t: "text", v: "2 * 3 * 4" }]);
    expect(parseInline("*not closed")).toEqual([{ t: "text", v: "*not closed" }]);
    expect(parseInline("a*b*c")).toEqual([{ t: "text", v: "a*b*c" }]);
  });

  it("allows punctuation after the closing marker", () => {
    expect(parseInline("_Never again._ Packing")).toEqual([
      { t: "i", c: [{ t: "text", v: "Never again." }] },
      { t: "text", v: " Packing" },
    ]);
    expect(parseInline("(*wow*)!")[1]).toEqual({ t: "b", c: [{ t: "text", v: "wow" }] });
  });

  it("links and trims trailing punctuation", () => {
    expect(parseInline("see https://x.io/a?b=1, ok")).toEqual([
      { t: "text", v: "see " },
      { t: "link", url: "https://x.io/a?b=1", v: "https://x.io/a?b=1" },
      { t: "text", v: ", ok" },
    ]);
    expect(parseInline("www.example.org.")[0]).toEqual({ t: "link", url: "https://www.example.org", v: "www.example.org" });
    expect(parseInline("xhttps://nope")).toEqual([{ t: "text", v: "xhttps://nope" }]);
  });

  it("does not format inside code or links", () => {
    expect(parseInline("`*raw*`")).toEqual([{ t: "code", v: "*raw*" }]);
    expect(parseInline("https://a.io/_x_/y")).toEqual([{ t: "link", url: "https://a.io/_x_/y", v: "https://a.io/_x_/y" }]);
  });

  it("resolves mentions only for listed jids", () => {
    const m = ["15550100000@s.whatsapp.net"];
    expect(parseInline("@15550100000 hi", m)[0]).toEqual({ t: "mention", jid: m[0], v: "15550100000" });
    expect(parseInline("@19998887777 hi", m)).toEqual([{ t: "text", v: "@19998887777 hi" }]);
    expect(parseInline("mail@15550100000", m)).toEqual([{ t: "text", v: "mail@15550100000" }]);
  });
});

describe("blocks", () => {
  it("splits paragraphs, lists, quotes and code fences", () => {
    const blocks = parse("Notes:\n- one\n- *two*\n\n> quoted\n> more\nafter\n1. first\n2. second\n```\nlet a = 1;\n```\nend");
    expect(blocks.map((b) => b.t)).toEqual(["p", "ul", "quote", "p", "ol", "pre", "p"]);
    const ul = blocks[1] as Extract<(typeof blocks)[number], { t: "ul" }>;
    expect(ul.items.length).toBe(2);
    expect(ul.items[1]).toEqual([{ t: "b", c: [{ t: "text", v: "two" }] }]);
    const pre = blocks[5] as Extract<(typeof blocks)[number], { t: "pre" }>;
    expect(pre.v).toBe("let a = 1;");
    const ol = blocks[4] as Extract<(typeof blocks)[number], { t: "ol" }>;
    expect(ol.start).toBe(1);
    expect(ol.items.length).toBe(2);
  });

  it("keeps an unclosed fence as text and preserves line breaks", () => {
    expect(parse("a ``` b")).toEqual([{ t: "p", c: [{ t: "text", v: "a " }] }, { t: "p", c: [{ t: "text", v: "``` b" }] }]);
    expect(parse("line one\nline two")).toEqual([{ t: "p", c: [{ t: "text", v: "line one\nline two" }] }]);
  });

  it("handles empty input", () => {
    expect(parse("")).toEqual([]);
    expect(parse("   \n ")).toEqual([]);
  });
});

describe("helpers", () => {
  it("detects emoji-only messages", () => {
    expect(jumboEmoji("😂")).toBe(1);
    expect(jumboEmoji("👍🏽 ❤️")).toBe(2);
    expect(jumboEmoji("👨‍👩‍👧‍👦")).toBe(1);
    expect(jumboEmoji("🇯🇵🇧🇷🇰🇪")).toBe(3);
    expect(jumboEmoji("😂😂😂😂")).toBe(0);
    expect(jumboEmoji("ok 👍")).toBe(0);
    expect(jumboEmoji("123")).toBe(0);
    expect(jumboEmoji("")).toBe(0);
  });

  it("strips formatting for previews", () => {
    expect(plain("*Don't* forget _this_ ~now~ `x`")).toBe("Don't forget this now x");
    expect(plain("line\nbreak")).toBe("line break");
    expect(plain("snake_case stays")).toBe("snake_case stays");
    expect(plain("@15550100000 hey", (u) => (u === "15550100000" ? "Sam" : undefined))).toBe("@Sam hey");
  });

  it("finds the first url", () => {
    expect(firstUrl("go to https://a.io/x. now")).toBe("https://a.io/x");
    expect(firstUrl("nothing here")).toBeNull();
  });
});
