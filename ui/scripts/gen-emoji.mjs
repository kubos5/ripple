// Builds a trimmed emoji table from emojibase-data (MIT) for the picker.
// Row: [emoji, name, search tags, group, skin-tone variants?]
import fs from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const data = require("emojibase-data/en/compact.json");
const rows = data
  .filter((e) => e.group !== undefined && e.group !== 2)
  .sort((a, b) => a.order - b.order)
  .map((e) => {
    const row = [e.unicode, e.label, (e.tags ?? []).slice(0, 8).join(" "), e.group];
    const skins = (e.skins ?? []).filter((s) => !s.hexcode.slice(e.hexcode.length + 1).includes("-1F3F") || true).slice(0, 5).map((s) => s.unicode);
    if (skins.length === 5) row.push(skins);
    return row;
  });
fs.mkdirSync("src/assets", { recursive: true });
fs.writeFileSync("src/assets/emoji.json", JSON.stringify(rows));
console.log(rows.length, "emoji,", fs.statSync("src/assets/emoji.json").size, "bytes");
