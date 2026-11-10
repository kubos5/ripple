// Small pure helpers for dates, sizes, names and colours.

const DAY = 86_400_000;

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekdayFmt = new Intl.DateTimeFormat(undefined, { weekday: "long" });
const weekdayShortFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dateFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long", year: "numeric" });
const dateShortFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const dateNumFmt = new Intl.DateTimeFormat(undefined, { day: "2-digit", month: "2-digit", year: "2-digit" });
const fullFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });

function startOfDay(ts: number) {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Whole days between two timestamps' calendar days (0 = same day). */
export function daysApart(a: number, b: number) {
  return Math.round((startOfDay(b) - startOfDay(a)) / DAY);
}

export function sameDay(a: number, b: number) {
  return startOfDay(a) === startOfDay(b);
}

export function clock(ts: number) {
  return timeFmt.format(ts);
}

/** Compact stamp for chat list rows. */
export function listTime(ts: number, now = Date.now()) {
  if (!ts) return "";
  const days = daysApart(ts, now);
  if (days <= 0) return clock(ts);
  if (days === 1) return "Yesterday";
  if (days < 7) return weekdayShortFmt.format(ts);
  return dateNumFmt.format(ts);
}

/** Label for the date separator between message groups. */
export function dayLabel(ts: number, now = Date.now()) {
  const days = daysApart(ts, now);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return weekdayFmt.format(ts);
  return dateFmt.format(ts);
}

export function fullStamp(ts: number) {
  return fullFmt.format(ts);
}

export function shortDate(ts: number) {
  return dateShortFmt.format(ts);
}

export function lastSeen(ts: number, now = Date.now()) {
  if (!ts) return "";
  const days = daysApart(ts, now);
  if (days <= 0) return `last seen today at ${clock(ts)}`;
  if (days === 1) return `last seen yesterday at ${clock(ts)}`;
  if (days < 7) return `last seen ${weekdayFmt.format(ts)} at ${clock(ts)}`;
  return `last seen ${dateShortFmt.format(ts)}`;
}

/** 75 → "1:15", 3723 → "1:02:03" */
export function duration(secs: number) {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${r}` : `${m}:${r}`;
}

export function fileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || Number.isInteger(v) ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function ephemeralLabel(secs: number) {
  if (!secs) return "Off";
  if (secs % 86_400 === 0) {
    const d = secs / 86_400;
    return d === 1 ? "24 hours" : `${d} days`;
  }
  if (secs % 3600 === 0) return `${secs / 3600} hours`;
  return `${Math.round(secs / 60)} minutes`;
}

export function muteLabel(until: number, now = Date.now()) {
  if (until === -1) return "Muted";
  if (until > now) return `Muted until ${daysApart(now, until) === 0 ? clock(until) : fullStamp(until)}`;
  return "";
}

export function initials(name: string) {
  const words = name.replace(/[^\p{L}\p{N}\s]/gu, " ").trim().split(/\s+/).filter(Boolean);
  if (!words.length) return "#";
  const first = [...words[0]][0] ?? "";
  const second = words.length > 1 ? ([...words[words.length - 1]][0] ?? "") : "";
  return (first + second).toUpperCase();
}

/** Stable 0–7 index from an id, for avatar and sender-name colours. */
export function tone(id: string) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h % 8;
}

/** "15551234567@s.whatsapp.net" → "+1 555 123 4567" */
export function phone(jid: string) {
  const user = jid.split("@")[0].split(":")[0];
  if (!/^\d{6,}$/.test(user) || jid.endsWith("@lid")) return user;
  if (user.startsWith("1") && user.length === 11) {
    return `+1 ${user.slice(1, 4)} ${user.slice(4, 7)} ${user.slice(7)}`;
  }
  const cc = user.length > 11 ? 3 : 2;
  const rest = user.slice(cc);
  const groups = rest.match(/.{1,4}/g) ?? [rest];
  return `+${user.slice(0, cc)} ${groups.join(" ")}`;
}

export function userPart(jid: string) {
  return jid.split("@")[0].split(":")[0];
}

export function isGroupJid(jid: string) {
  return jid.endsWith("@g.us");
}

export function plural(n: number, one: string, many = one + "s") {
  return `${n} ${n === 1 ? one : many}`;
}

export function clamp(v: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, v));
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}
