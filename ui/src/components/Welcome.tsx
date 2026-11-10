// First-run and pairing screens.

import { For, Match, Show, Switch, createSignal } from "solid-js";
import { act, state } from "../lib/state";
import { QR } from "./Modals";
import { Icon } from "./ui";

function Disclaimer() {
  return (
    <p class="fineprint">
      <Icon name="info" />
      Ripple is an independent client and is not affiliated with WhatsApp or Meta. Unofficial clients can go against WhatsApp's terms of service, so link an account at your own risk.
    </p>
  );
}

function Intro(props: { onPhone: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const pair = async () => {
    setBusy(true);
    await act("pair", { phone: null });
    setBusy(false);
  };
  return (
    <div class="welcome-card">
      <h2 class="display">Link your account</h2>
      <p class="lead">Ripple works as a linked device, like WhatsApp Web. Your chats sync over an end-to-end encrypted connection and your phone doesn't need to stay online.</p>
      <button class="btn primary lg block" disabled={busy()} onClick={() => void pair()}>
        <Show when={busy()} fallback={<Icon name="qr-code" />}><div class="spinner" /></Show>Link with QR code
      </button>
      <button class="btn lg block" onClick={props.onPhone}><Icon name="device-mobile" />Link with phone number</button>
      <button class="btn ghost block" onClick={() => void act("demoStart")}><Icon name="sparkle" />Explore the demo first</button>
      <Disclaimer />
    </div>
  );
}

function PhoneForm(props: { back: () => void }) {
  const [phone, setPhone] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const digits = () => phone().replace(/[^\d]/g, "");
  const go = async () => {
    setBusy(true);
    await act("pair", { phone: digits() });
    setBusy(false);
  };
  return (
    <div class="welcome-card">
      <h2 class="display">Enter your number</h2>
      <p class="lead">Use the full international format, including the country code. You'll get a code to type into WhatsApp on your phone.</p>
      <input class="field" style={{ height: "48px", "font-size": "var(--fs-17)" }} inputmode="tel" placeholder="+44 7700 900123" value={phone()} ref={(el) => setTimeout(() => el.focus(), 40)}
        onInput={(e) => setPhone(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && digits().length >= 8 && void go()} />
      <button class="btn primary lg block" disabled={digits().length < 8 || busy()} onClick={() => void go()}>
        <Show when={busy()}><div class="spinner" /></Show>Get code
      </button>
      <button class="btn ghost block" onClick={props.back}>Back</button>
    </div>
  );
}

function Pairing(props: { qr: string | null; code: string | null; expires: number }) {
  const left = () => Math.max(1, props.expires - Date.now());
  return (
    <div class="welcome-card" style={{ "align-items": "stretch" }}>
      <Show when={props.code} fallback={
        <>
          <h2 class="display">Scan to link</h2>
          <ol class="steps">
            <li><span>Open <b>WhatsApp</b> on your phone</span></li>
            <li><span>Go to <b>Settings › Linked devices</b> and tap <b>Link a device</b></span></li>
            <li><span>Point your phone at this code</span></li>
          </ol>
          <div class="qr-frame">
            <Show when={props.qr} keyed fallback={<div class="qr-wait"><div class="spinner" /></div>}>
              {(qr) => <QR text={qr} />}
            </Show>
          </div>
          <Show when={props.qr} keyed>{(_q) => <div class="qr-timer"><i style={{ "animation-duration": `${left()}ms` }} /></div>}</Show>
        </>
      }>
        {(code) => (
          <>
            <h2 class="display">Enter this code</h2>
            <ol class="steps">
              <li><span>Open <b>WhatsApp</b> on your phone</span></li>
              <li><span>Go to <b>Settings › Linked devices › Link a device</b></span></li>
              <li><span>Tap <b>Link with phone number instead</b> and type the code</span></li>
            </ol>
            <div class="pair-code">
              <For each={[...code().replace(/-/g, "")]}>
                {(ch, i) => (
                  <>
                    <Show when={i() === 4}><span class="dash">–</span></Show>
                    <span style={{ "--i": i() }}>{ch}</span>
                  </>
                )}
              </For>
            </div>
          </>
        )}
      </Show>
      <button class="btn ghost block" onClick={() => void act(state.demo ? "demoTrigger" : "logout", state.demo ? { what: "offline" } : {})} style={{ display: state.demo ? "none" : undefined }}>Cancel</button>
      <p class="fineprint" style={{ "text-align": "center" }}>Keep this window open until linking finishes.</p>
    </div>
  );
}

export function Welcome() {
  const [phoneMode, setPhoneMode] = createSignal(false);
  const s = () => state.session;
  return (
    <div class="welcome">
      <div class="welcome-art drag">
        <div class="rings"><i /><i /><i /><i /><b /></div>
        <h1 class="display">ripple</h1>
        <p>A fast, quiet desktop home for your WhatsApp conversations.</p>
      </div>
      <div class="welcome-main">
        <Switch>
          <Match when={s().state === "pairing" && s()}>
            {(p) => { const v = p() as Extract<typeof state.session, { state: "pairing" }>; return <Pairing qr={v.qr} code={v.code} expires={v.expires} />; }}
          </Match>
          <Match when={s().state === "syncing" && s()}>
            {(p) => (
              <div class="welcome-card">
                <h2 class="display">Syncing your chats</h2>
                <p class="lead">Messages are being decrypted on this computer. This usually takes less than a minute.</p>
                <div class="sync-bar"><i style={{ width: `${Math.max(4, (p() as { progress: number }).progress)}%` }} /></div>
                <p class="fineprint tnum">{(p() as { progress: number }).progress}%</p>
              </div>
            )}
          </Match>
          <Match when={s().state === "loggedOut" && s()}>
            {(p) => (
              <div class="welcome-card">
                <h2 class="display">You've been logged out</h2>
                <p class="lead">{(p() as { reason: string }).reason || "This device was unlinked from your account."}</p>
                <button class="btn primary lg block" onClick={() => void act("pair", { phone: null })}><Icon name="qr-code" />Link again</button>
                <button class="btn ghost block" onClick={() => void act("demoStart")}>Explore the demo</button>
              </div>
            )}
          </Match>
          <Match when={phoneMode()}><PhoneForm back={() => setPhoneMode(false)} /></Match>
          <Match when={true}><Intro onPhone={() => setPhoneMode(true)} /></Match>
        </Switch>
      </div>
    </div>
  );
}
