import { Button, Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode, NativePointerBoundaryEvent } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true);
    const [counts, setCounts] = createSignal({
      regionEnter: 0,
      regionLeave: 0,
      aEnter: 0,
      aLeave: 0,
      bEnter: 0,
      bLeave: 0,
    });
    const [last, setLast] = createSignal("");
    const [captured, setCaptured] = createSignal("");
    const [retained, setRetained] = createSignal("");
    const [authority, setAuthority] = createSignal("");
    let region: NativeNode | undefined,
      a: NativeNode | undefined,
      held: NativePointerBoundaryEvent | undefined;
    const record = (name: keyof ReturnType<typeof counts>, event: NativePointerBoundaryEvent) => {
      setCounts(current => ({ ...current, [name]: current[name] + 1 }));
      held = event;
      setLast(`${event.type}|${event.position.x},${event.position.y}|${event.pointerKind}`);
    };
    return (
      <>
        <box style="position: fixed; inset: 0; background-color: #ffffff" />
        <Button
          testId="toggle"
          style="position: fixed; left: 20px; top: 20px; width: 160px; height: 36px; font-size: 14px; background-color: #e5e7eb; color: #111827"
          onClick={() => {
            if (held) {
              setRetained(`${held.type}|${held.position.x},${held.position.y}`);
              let expired = false;
              try {
                held.stopPropagation();
              } catch (error) {
                expired = error instanceof ReferenceError;
              }
              setAuthority(expired ? "expired" : "active");
            }
            setShown(value => !value);
          }}
        >
          Toggle region
        </Button>
        <Show when={shown()}>
          <box
            testId="region"
            ref={node => {
              region = node;
            }}
            style="position: fixed; left: 100px; top: 100px; width: 240px; height: 140px; box-sizing: border-box; padding: 10px; display: flex; gap: 10px; background-color: #e5e7eb"
            onPointerEnter={event => record("regionEnter", event)}
            onPointerLeave={event => record("regionLeave", event)}
            onPointerEnterCapture={event => {
              if (event.target === a)
                setCaptured(`${event.phase}|${event.currentTarget === region}`);
            }}
          >
            <box
              testId="a"
              ref={node => {
                a = node;
              }}
              style="width: 90px; height: 60px; flex-shrink: 0; background-color: #0f766e"
              onPointerEnter={event => record("aEnter", event)}
              onPointerLeave={event => record("aLeave", event)}
            />
            <box
              testId="b"
              style="width: 90px; height: 60px; flex-shrink: 0; background-color: #374151"
              onPointerEnter={event => record("bEnter", event)}
              onPointerLeave={event => record("bLeave", event)}
            />
          </box>
        </Show>
        <column style="position: fixed; left: 380px; top: 20px; width: 360px; gap: 8px; font-size: 14px; color: #111827">
          <text>Region / first child / second child</text>
          <text testId="region-count">{`${counts().regionEnter}/${counts().regionLeave}`}</text>
          <text testId="a-count">{`${counts().aEnter}/${counts().aLeave}`}</text>
          <text testId="b-count">{`${counts().bEnter}/${counts().bLeave}`}</text>
          <text testId="last">{last()}</text>
          <text testId="capture">{captured()}</text>
          <text testId="retained">{retained()}</text>
          <text testId="authority">{authority()}</text>
        </column>
      </>
    );
  }, root);
}
