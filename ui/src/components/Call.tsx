// Incoming-call banner. Ripple can't answer calls, so it offers to decline.

import { Show } from "solid-js";
import { Portal } from "solid-js/web";
import { act, contactName, state } from "../lib/state";
import { Avatar, Icon } from "./ui";

export function CallBanner() {
  return (
    <Show when={state.call} keyed>
      {(c) => (
        <Portal>
          <div class="call" role="alertdialog" aria-label="Incoming call">
            <div class="ringing"><Avatar id={c.from} size={46} /></div>
            <div class="grow">
              <div class="c-name truncate">{contactName(c.from, { you: false })}</div>
              <div class="c-sub">Incoming {c.video ? "video" : "voice"} call · answer on your phone</div>
            </div>
            <button class="decline" aria-label="Decline" data-tip="Decline" onClick={() => void act("rejectCall", { id: c.id, from: c.from })}>
              <Icon name="phone-x-fill" />
            </button>
          </div>
        </Portal>
      )}
    </Show>
  );
}
