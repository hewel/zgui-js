import { Show, createSignal, onCleanup, onSettled, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { readTextAsset } from "@hewel/zgui-services";
import type { HostRequest } from "@hewel/zgui-services";

function AsyncStatus() {
  const [timerStatus, setTimerStatus] = createSignal("Timer: pending");
  const [frameStatus, setFrameStatus] = createSignal("Frame: pending");
  const [promiseStatus, setPromiseStatus] = createSignal("Promise: pending");
  const [serviceStatus, setServiceStatus] = createSignal("Service: waiting for timer");
  const [layoutReady, setLayoutReady] = createSignal(false);
  let active = true;
  let timer: number | undefined;
  let frame: number | undefined;
  let request: HostRequest<string> | undefined;

  onCleanup(() => {
    active = false;
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (frame !== undefined) {
      cancelAnimationFrame(frame);
      frame = undefined;
    }
    request?.cancel();
  });

  onSettled(() => {
    frame = requestAnimationFrame(() => {
      frame = undefined;
      setFrameStatus("Frame: fired");
      void Promise.resolve().then(() => {
        if (!active) return;
        setPromiseStatus("Promise: layout updated");
        setLayoutReady(true);
      });
    });
    timer = setTimeout(() => {
      timer = undefined;
      setTimerStatus("Timer: fired");
      setServiceStatus("Service: reading");
      request = readTextAsset("assets/message.txt");
      void request.promise.then(message => {
        request = undefined;
        if (active) setServiceStatus(`Service: ${message.trim()}`);
      }, error => {
        request = undefined;
        if (active) setServiceStatus(`Service: failed (${error instanceof Error ? error.message : String(error)})`);
      });
    }, 50);
  });

  return <column style={{ gap: "8px" }}>
    <text>Async status</text>
    <text>{timerStatus()}</text>
    <text>{frameStatus()}</text>
    <text>{promiseStatus()}</text>
    <text>{serviceStatus()}</text>
    <Show when={layoutReady()}><text class="promise-layout-row" style={{ height: "32px" }}>Promise layout row</text></Show>
  </column>;
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <AsyncStatus />, root);
}
