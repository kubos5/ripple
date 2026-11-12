# Ripple

A fast, lightweight desktop client for WhatsApp, written in Rust.

Ripple links to your account as a companion device (like WhatsApp Web) and keeps
your chats in a local, searchable database. The protocol, encryption, storage and
media handling are all Rust; the interface is drawn by the operating system's own
webview through [Tauri](https://tauri.app), so there is no bundled browser.

> **Ripple is an independent project. It is not affiliated with, endorsed by or
> supported by WhatsApp or Meta.** Unofficial clients can go against WhatsApp's
> terms of service, and using one may put your account at risk of restriction.
> Link an account at your own risk. You can explore everything in the built-in
> demo without linking anything.

## What it does

**Chats.** Pinned, archived, muted and favourite chats, unread and group filters,
drafts, typing previews, global full-text search across every message, and a
command palette (`Cmd/Ctrl+K`) to jump to any chat, person, message or action.

**Messages.** Text with WhatsApp formatting (bold, italic, strikethrough, code,
lists, quotes), replies, mentions, link previews, reactions, edits, delete for
everyone or for yourself, forwarding, stars, pinned messages, message info,
multi-select, disappearing messages, read receipts and typing indicators.

**Media.** Photos, videos, albums, GIFs, documents, audio, voice notes (record,
waveform, 1×/1.5×/2× playback), stickers (recents and favourites synced with
WhatsApp on your phone, create from any image), view-once media, a lightbox with zoom, and a per-chat media gallery.

**More message types.** Polls, events with RSVP, locations, contact cards.

**Groups and communities.** Create groups, manage members and admins, group
settings, invite links with QR codes, join requests; community home with the
announcements channel and subgroups, creating communities, linking groups.

**Desktop.** Native notifications, unread badge, tray icon (Windows and Linux),
launch at login, light and dark themes, accent colours, a choice of fonts,
adjustable text size, reduced motion.

**Not included yet:** Status, Channels, and answering voice or video calls
(incoming calls show a banner you can decline).

## Layout

```
crates/ripple-core     All logic: models, SQLite store, WhatsApp session, media, demo
crates/ripple-bridge   Dev-only WebSocket bridge so the UI can run in a browser tab
ui/                    SolidJS + TypeScript interface
ui/src-tauri           Desktop shell (window, tray, notifications, media protocol)
```

The UI talks to the core through one request type and one event stream
(`crates/ripple-core/src/api.rs`). Behind that sits either the real WhatsApp
backend (`src/wa`, built on [`whatsapp-rust`](https://github.com/oxidezap/whatsapp-rust))
or a scripted demo backend (`src/demo`) that exercises the same code paths.

## Running it

You need Rust 1.95 or newer and Node 20 or newer. On Linux, also install the
[Tauri system dependencies](https://tauri.app/start/prerequisites/) plus
`libasound2-dev`.

```bash
cd ui && npm install
```

Run the desktop app in development:

```bash
cd ui && npx tauri dev
```

Start it straight in demo mode (no account needed):

```bash
cd ui && RIPPLE_DEMO=1 npx tauri dev
```

Build installers for your platform (output in `target/release/bundle`):

```bash
cd ui && npx tauri build
```

### Working on the UI in a browser

The bridge serves the same API over a localhost WebSocket, backed by the demo:

```bash
cargo run -p ripple-bridge
```

```bash
cd ui && npm run dev
```

Then open http://127.0.0.1:1420. Set `RIPPLE_DEMO_QUIET=1` to stop the demo's
scripted incoming messages, or `RIPPLE_DEMO_STRESS=1` to load a 50,000-message
chat and 2,000 extra chats.

## Tests

```bash
cargo test -p ripple-core -p ripple-bridge
```

```bash
cd ui && npm run typecheck && npm test
```

## Performance

Measured on an Apple Silicon Mac with the release build (November 2026):

| | |
|---|---|
| Installer (`.dmg`) | 12 MB |
| Cold start to a visible window | about 0.45 s |
| Memory, demo data loaded | 33 MB for Ripple itself, about 115 MB including the system webview's helper processes |
| CPU when idle | 0% |
| Open a chat with 50,000 messages | about 60 ms |
| Load an older page of messages | 25–40 ms in the UI, under 1 ms in the database |
| Search 52,000 messages | 10–30 ms |

Chat and message lists are virtualised, so only what is on screen is rendered
(about 30 message rows and a dozen chat rows, however large the history).

## Known limitations

- Windows and Linux builds are produced by CI but have not been run by hand yet.
- Live WhatsApp behaviour depends on `whatsapp-rust`, which is pre-1.0; expect
  occasional breakage when WhatsApp changes its protocol. Ripple pins an
  unreleased upstream commit, because sticker sync is not in a release yet.
- Votes on polls that arrive through history sync (sent before you linked) are
  not shown; votes cast afterwards are.
- GIF files are sent as documents (WhatsApp GIFs are short MP4 videos, and
  Ripple does not transcode video).
- Videos are sent as they are; there is no compression step.

## Where your data lives

Everything stays on your computer, in the app data directory
(`~/Library/Application Support/app.ripple.desktop` on macOS,
`%APPDATA%\app.ripple.desktop` on Windows, `~/.local/share/app.ripple.desktop`
on Linux): `live/app.db` (messages and search index), `live/wa.db` (the linked
device's encryption keys) and `live/media/` (downloaded files). Logging out
removes all three. Ripple makes no network requests other than to WhatsApp,
except to fetch a link preview for a URL you type (this can be turned off in
Settings).

## Credits

Protocol: [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) (MIT).
Icons: [Material Symbols](https://fonts.google.com/icons) (Apache 2.0). Emoji
data: [emojibase](https://emojibase.dev) (MIT). Ripple uses your system font by
default; the optional Hanken Grotesk, IBM Plex Sans and Atkinson Hyperlegible
Next typefaces are bundled under the SIL Open Font License.
