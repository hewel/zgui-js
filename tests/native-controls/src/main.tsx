import { Button, createSignal, renderApp } from "@zgui/ui";
import type { NativeEvent, NativeNode } from "@zgui/ui";

let nextMode: (() => void) | undefined;

function Controls() {
  const [mode, setMode] = createSignal(0);
  const [disabledInKeydown, setDisabledInKeydown] = createSignal(false);
  let disableKeydowns = 0;
  let primary = 0;
  let guarded = 0;
  let disabled = 0;
  let replacement = 0;
  let expired = 0;
  let log: string[] = [];
  let retained: NativeEvent | undefined;
  let retainedTarget: NativeNode | undefined;
  const [report, setReport] = createSignal("");

  function publish() {
    setReport(JSON.stringify({ mode: mode(), primary, guarded, disabled, replacement, expired, log, disabledInKeydown: disabledInKeydown(), disableKeydowns }));
  }
  function checkExpired() {
    if (!retained) throw new Error("No native event was retained");
    if (retained.type !== "click" || retained.phase !== "target" || retained.target !== retainedTarget) {
      throw new Error("Retained native snapshot changed after dispatch");
    }
    for (const cancel of [retained.preventDefault, retained.stopPropagation, retained.stopImmediatePropagation]) {
      let rejected = false;
      try { cancel(); }
      catch (error) {
        if (!(error instanceof ReferenceError) || !error.message.includes("expired-dispatch")) throw error;
        rejected = true;
        expired++;
      }
      if (!rejected) throw new Error("Retained cancellation remained active after its handler returned");
    }
  }
  function record(name: string, event: NativeEvent) {
    log.push(`${name}:${event.phase}`);
    publish();
  }
  function primaryCapture(event: NativeEvent) {
    retained = event;
    retainedTarget = event.target;
    record("primary-capture", event);
    if (mode() === 4) event.stopPropagation();
    if (mode() === 5) event.stopImmediatePropagation();
  }
  function primaryClick(event: NativeEvent) {
    // Even the next handler in the SAME native dispatch cannot use this authority.
    checkExpired();
    primary++;
    record("primary-bubble", event);
  }
  function replacementClick(event: NativeEvent) {
    checkExpired();
    replacement++;
    record("replacement-bubble", event);
  }
  nextMode = () => {
    checkExpired();
    log = [];
    setMode(value => value + 1);
    publish();
  };
  publish();

  return <column class="controls"
    onClickCapture={event => record("outer-capture", event)}
    onClick={event => record("outer-bubble", event)}>
    <column class="middle"
      onClickCapture={event => {
        record("middle-capture", event);
        if (mode() === 6) event.stopPropagation();
      }}
      onClick={event => record("middle-bubble", event)}>
      <Button class="primary" disabled={mode() === 3 || disabledInKeydown()}
        onClickCapture={primaryCapture}
        onClick={mode() === 8 ? null : mode() === 7 ? replacementClick : primaryClick}
        onKeyDown={event => {
          if (mode() === 1) event.preventDefault();
          if (mode() === 9) {
            // Do not cancel: the native default must recheck this synchronous mutation.
            disableKeydowns++;
            setDisabledInKeydown(true);
            publish();
          }
        }}>
        Primary
      </Button>
      <Button class="guarded"
        onPointerDown={event => { if (mode() === 2) event.preventDefault(); }}
        onPointerUp={event => { if (mode() === 3) event.preventDefault(); }}
        onClick={() => { guarded++; publish(); }}>
        Guarded
      </Button>
      <Button class="disabled" disabled
        onClick={() => { disabled++; publish(); }}>
        Disabled
      </Button>
    </column>
    <text class="report">{report()}</text>
  </column>;
}

export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  const dispose = renderApp(() => <Controls />, root);
  return () => { nextMode = undefined; dispose(); };
}
export function advance(): void {
  if (!nextMode) throw new Error("Native controls fixture is not mounted");
  nextMode();
}
