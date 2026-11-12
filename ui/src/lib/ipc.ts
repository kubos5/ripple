// One API, two transports: Tauri IPC inside the desktop app, a localhost
// WebSocket to `ripple-bridge` when the UI runs in a plain browser tab.

import type { UiEvent } from "./types";

type Listener = (events: UiEvent[]) => void;

interface Transport {
  call(req: Record<string, unknown>): Promise<unknown>;
  listen(fn: Listener): void;
  mediaUrl(token: string, wav: boolean): string;
}

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function tauriTransport(): Transport {
  const core = import("@tauri-apps/api/core");
  return {
    async call(req) {
      const { invoke } = await core;
      return invoke("ripple", { req });
    },
    listen(fn) {
      void core.then(({ invoke, Channel }) => {
        const channel = new Channel<UiEvent[]>();
        channel.onmessage = fn;
        void invoke("subscribe", { channel });
      });
    },
    mediaUrl(token, wav) {
      // macOS/Linux use a custom scheme; Windows maps it onto http://<scheme>.localhost.
      const windows = navigator.userAgent.includes("Windows");
      const base = windows ? "http://media.localhost/" : "media://localhost/";
      return base + token.split("/").map(encodeURIComponent).join("/") + (wav ? "?wav=1" : "");
    },
  };
}

function bridgeTransport(): Transport {
  const host = `127.0.0.1:${import.meta.env.VITE_BRIDGE_PORT ?? 1421}`;
  let socket: WebSocket | null = null;
  let nextId = 1;
  let listener: Listener = () => {};
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  let queue: string[] = [];
  let everOpened = false;

  const connect = () => {
    socket = new WebSocket(`ws://${host}/ws`);
    socket.onopen = () => {
      if (everOpened) listener([{ type: "reset" }]);
      everOpened = true;
      queue.forEach((m) => socket!.send(m));
      queue = [];
    };
    socket.onmessage = (ev) => {
      const msg = JSON.parse(ev.data as string);
      if (msg.events) return listener(msg.events as UiEvent[]);
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      if (msg.ok) p.resolve(msg.value);
      else p.reject(new Error(msg.error));
    };
    socket.onclose = () => {
      pending.forEach((p) => p.reject(new Error("Lost connection to the Ripple bridge")));
      pending.clear();
      setTimeout(connect, 800);
    };
  };
  connect();

  return {
    call(req) {
      return new Promise((resolve, reject) => {
        const id = nextId++;
        pending.set(id, { resolve, reject });
        const payload = JSON.stringify({ id, req });
        if (socket && socket.readyState === WebSocket.OPEN) socket.send(payload);
        else queue.push(payload);
      });
    },
    listen(fn) {
      listener = fn;
    },
    mediaUrl(token, wav) {
      return `http://${host}/media/${token}${wav ? "?wav=1" : ""}`;
    },
  };
}

const transport: Transport = inTauri ? tauriTransport() : bridgeTransport();

/** Sends a command to the core. Rejects with a human-readable `Error`. */
export function call<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return transport.call({ cmd, ...args }).catch((e) => {
    throw e instanceof Error ? e : new Error(String(e));
  }) as Promise<T>;
}

export function listen(fn: Listener) {
  transport.listen(fn);
}

let oggSupport: boolean | null = null;
function canPlayOgg() {
  if (oggSupport === null) {
    oggSupport = document.createElement("audio").canPlayType('audio/ogg; codecs="opus"') !== "";
  }
  return oggSupport;
}

/** URL for a media token. Ogg Opus audio is transparently served as WAV where the webview cannot play it. */
export function mediaUrl(token: string | null | undefined, mime = ""): string {
  if (!token) return "";
  const wav = mime.startsWith("audio/ogg") && !canPlayOgg();
  return transport.mediaUrl(token, wav);
}
