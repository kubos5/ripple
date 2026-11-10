// WhatsApp-style text formatting: *bold*, _italic_, ~strike~, `code`,
// ```blocks```, "> quotes", bullet and numbered lists, links and @mentions.

export type Inline =
  | { t: "text"; v: string }
  | { t: "b" | "i" | "s"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "link"; url: string; v: string }
  | { t: "mention"; jid: string; v: string };

export type Block =
  | { t: "p"; c: Inline[] }
  | { t: "quote"; c: Inline[] }
  | { t: "ul"; items: Inline[][] }
  | { t: "ol"; items: Inline[][]; start: number }
  | { t: "pre"; v: string };

const MARKS: Record<string, "b" | "i" | "s"> = { "*": "b", _: "i", "~": "s" };
const WORD = /[\p{L}\p{N}]/u;
const URL_RE = /^(?:https?:\/\/|www\.)[^\s<>"]+/i;
const TRAILING = /[.,;:!?)\]}'"…]+$/;

function isWord(ch: string | undefined) {
  return !!ch && WORD.test(ch);
}

export function parseInline(s: string, mentions: string[] = []): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  const flush = () => {
    if (buf) out.push({ t: "text", v: buf });
    buf = "";
  };
  let i = 0;
  while (i < s.length) {
    const ch = s[i];
    const prev = s[i - 1];

    if (ch === "`") {
      const end = s.indexOf("`", i + 1);
      if (end > i + 1 && !s.slice(i + 1, end).includes("\n")) {
        flush();
        out.push({ t: "code", v: s.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }

    if ((ch === "h" || ch === "w" || ch === "H" || ch === "W") && !isWord(prev)) {
      const m = URL_RE.exec(s.slice(i));
      if (m) {
        const raw = m[0].replace(TRAILING, "");
        if (raw.length > 7 || raw.toLowerCase().startsWith("www.")) {
          flush();
          out.push({ t: "link", url: /^https?:/i.test(raw) ? raw : `https://${raw}`, v: raw });
          i += raw.length;
          continue;
        }
      }
    }

    if (ch === "@" && !isWord(prev)) {
      const m = /^@(\d{5,20})/.exec(s.slice(i));
      const jid = m && mentions.find((j) => j.split("@")[0].split(":")[0] === m[1]);
      if (m && jid) {
        flush();
        out.push({ t: "mention", jid, v: m[1] });
        i += m[0].length;
        continue;
      }
    }

    const mark = MARKS[ch];
    if (mark && !isWord(prev) && s[i + 1] !== undefined && !/\s/.test(s[i + 1]) && s[i + 1] !== ch) {
      // Find the closing marker: preceded by non-space, followed by a non-word character.
      let end = -1;
      for (let j = i + 2; j < s.length; j++) {
        if (s[j] === "\n") break;
        if (s[j] === ch && !/\s/.test(s[j - 1]) && !isWord(s[j + 1])) {
          end = j;
          break;
        }
      }
      if (end > 0) {
        flush();
        out.push({ t: mark, c: parseInline(s.slice(i + 1, end), mentions) });
        i = end + 1;
        continue;
      }
    }

    buf += ch;
    i++;
  }
  flush();
  return out;
}

export function parse(text: string, mentions: string[] = []): Block[] {
  const blocks: Block[] = [];
  const parts = text.split("```");
  parts.forEach((part, idx) => {
    const fenced = idx % 2 === 1 && idx < parts.length - 1;
    if (fenced) {
      blocks.push({ t: "pre", v: part.replace(/^\n/, "").replace(/\n$/, "") });
      return;
    }
    // An unclosed fence keeps its backticks as literal text.
    const src = idx % 2 === 1 ? "```" + part : part;
    const lines = src.split("\n");
    let para: string[] = [];
    const flushPara = () => {
      const joined = para.join("\n");
      if (joined.trim()) blocks.push({ t: "p", c: parseInline(joined.replace(/^\n+|\n+$/g, ""), mentions) });
      para = [];
    };
    for (let n = 0; n < lines.length; n++) {
      const line = lines[n];
      const quote = /^> ?(.*)$/.exec(line);
      const bullet = /^[-*] (.+)$/.exec(line);
      const number = /^(\d{1,3})\. (.+)$/.exec(line);
      const last = blocks[blocks.length - 1];
      if (quote && line.startsWith(">")) {
        flushPara();
        const prevLine = n > 0 ? lines[n - 1] : "";
        if (last?.t === "quote" && prevLine.startsWith(">")) {
          last.c.push({ t: "text", v: "\n" }, ...parseInline(quote[1], mentions));
        } else {
          blocks.push({ t: "quote", c: parseInline(quote[1], mentions) });
        }
      } else if (bullet) {
        flushPara();
        const item = parseInline(bullet[1], mentions);
        if (last?.t === "ul" && /^[-*] /.test(lines[n - 1] ?? "")) last.items.push(item);
        else blocks.push({ t: "ul", items: [item] });
      } else if (number) {
        flushPara();
        const item = parseInline(number[2], mentions);
        if (last?.t === "ol" && /^\d{1,3}\. /.test(lines[n - 1] ?? "")) last.items.push(item);
        else blocks.push({ t: "ol", items: [item], start: Number(number[1]) });
      } else {
        para.push(line);
      }
    }
    flushPara();
  });
  return blocks;
}

const EMOJI_ONLY =
  /^(?:(?:\p{Extended_Pictographic}|\p{Regional_Indicator}{2}|[#*0-9]️?⃣)(?:️|‍|\p{Emoji_Modifier}|\p{Extended_Pictographic}|[\u{E0020}-\u{E007F}])*|\s)+$/u;

const segmenter = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter(undefined, { granularity: "grapheme" }) : null;

/** Number of emoji when the text is nothing but 1–3 emoji (rendered large); otherwise 0. */
export function jumboEmoji(text: string): number {
  const t = text.trim();
  if (!t || t.length > 40 || !EMOJI_ONLY.test(t)) return 0;
  const count = segmenter ? [...segmenter.segment(t)].filter((s) => s.segment.trim()).length : [...t].length;
  return count <= 3 ? count : 0;
}

/** Strips formatting markers for one-line previews. */
export function plain(text: string, names?: (user: string) => string | undefined): string {
  let t = text
    .replace(/```([\s\S]*?)```/g, "$1")
    .replace(/`([^`\n]+)`/g, "$1")
    .replace(/(^|[^\p{L}\p{N}])([*_~])(?=\S)([^\n]*?\S)\2(?![\p{L}\p{N}])/gu, "$1$3")
    .replace(/(^|[^\p{L}\p{N}])([*_~])(?=\S)([^\n]*?\S)\2(?![\p{L}\p{N}])/gu, "$1$3")
    .replace(/\s*\n\s*/g, " ");
  if (names) t = t.replace(/@(\d{5,20})/g, (m, u) => (names(u) ? `@${names(u)}` : m));
  return t;
}

export function firstUrl(text: string): string | null {
  const m = /(?:^|\s)(https?:\/\/[^\s<>"]+)/i.exec(text);
  return m ? m[1].replace(TRAILING, "") : null;
}
