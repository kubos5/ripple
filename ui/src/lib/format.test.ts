import { describe, expect, it } from "vitest";
import { dayLabel, daysApart, duration, ephemeralLabel, fileSize, initials, listTime, phone, tone } from "./format";

describe("format helpers", () => {
  it("formats phone numbers by country code length", () => {
    expect(phone("15551234567@s.whatsapp.net")).toBe("+1 555 123 4567");
    expect(phone("447700900123@s.whatsapp.net")).toBe("+44 770 090 0123");
    expect(phone("351912345678:4@s.whatsapp.net")).toBe("+351 912 345 678");
    expect(phone("79161234567@s.whatsapp.net")).toBe("+7 916 123 4567");
    expect(phone("123456789@lid")).toBe("123456789");
  });

  it("formats durations and sizes", () => {
    expect(duration(0)).toBe("0:00");
    expect(duration(75)).toBe("1:15");
    expect(duration(3723)).toBe("1:02:03");
    expect(fileSize(512)).toBe("512 B");
    expect(fileSize(1536)).toBe("1.5 KB");
    expect(fileSize(5 * 1024 * 1024)).toBe("5 MB");
    expect(ephemeralLabel(0)).toBe("Off");
    expect(ephemeralLabel(86_400)).toBe("24 hours");
    expect(ephemeralLabel(7 * 86_400)).toBe("7 days");
  });

  it("labels days relative to now", () => {
    const now = new Date(2026, 10, 11, 12, 0).getTime();
    expect(dayLabel(now - 3_600_000, now)).toBe("Today");
    expect(dayLabel(now - 20 * 3_600_000, now)).toBe("Yesterday");
    expect(daysApart(now - 3 * 86_400_000, now)).toBe(3);
    expect(listTime(now - 26 * 3_600_000, now)).toBe("Yesterday");
    expect(listTime(0, now)).toBe("");
  });

  it("derives initials and stable tones", () => {
    expect(initials("Maya Okafor")).toBe("MO");
    expect(initials("dad")).toBe("D");
    expect(initials("Weekend Hikers 🥾")).toBe("WH");
    expect(initials("")).toBe("#");
    expect(tone("a@b")).toBe(tone("a@b"));
    expect(tone("x")).toBeLessThan(8);
  });
});
