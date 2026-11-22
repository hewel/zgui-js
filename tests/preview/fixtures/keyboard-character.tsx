import { Button, TextField, createSignal, renderApp } from "@hewel/zgui";
import type { NativeKeyEvent, NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
const points = (text: string | null) =>
  text === null ? "none" : Array.from(text, value => value.codePointAt(0)!.toString(16)).join(".");
const describe = (event: NativeKeyEvent) =>
  `${points(event.key)}|${points(event.character)}|${points(event.keyWithoutModifiers)}|${Number(event.modifiers.shift)}${Number(event.modifiers.control)}${Number(event.modifiers.alt)}${Number(event.modifiers.meta)}`;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [down, setDown] = createSignal("");
    const [up, setUp] = createSignal("");
    const [clicks, setClicks] = createSignal(0);
    const [events, setEvents] = createSignal(0);
    const [retained, setRetained] = createSignal("");
    const [authority, setAuthority] = createSignal("");
    let held: NativeKeyEvent | undefined;
    const handlers = {
      onKeyDown(event: NativeKeyEvent) {
        held = event;
        setDown(describe(event));
        setEvents(value => value + 1);
        if (event.character === "x") event.preventDefault();
      },
      onKeyUp(event: NativeKeyEvent) {
        setUp(describe(event));
      },
    };
    const buttonStyle =
      "width: 180px; height: 36px; background-color: #e5e7eb; color: #111827; font-size: 14px";
    return (
      <column style="padding: 20px; gap: 12px; min-height: 100%; background-color: #ffffff; color: #111827; font-size: 14px">
        <Button
          testId="target"
          style={buttonStyle}
          {...handlers}
          onClick={() => setClicks(value => value + 1)}
        >
          Keyboard target
        </Button>
        <Button
          testId="inspect"
          style={buttonStyle}
          onClick={() => {
            if (!held) return;
            setRetained(describe(held));
            let expired = false;
            try {
              held.preventDefault();
            } catch (error) {
              expired = error instanceof ReferenceError;
            }
            setAuthority(expired ? "expired" : "active");
          }}
        >
          Inspect retained key
        </Button>
        <TextField
          testId="editor"
          defaultValue=""
          ariaLabel="Keyboard editor"
          style="width: 300px; height: 36px"
          {...handlers}
        />
        <text>Down / Up / Activations / Key events</text>
        <text testId="down">{down()}</text>
        <text testId="up">{up()}</text>
        <text testId="clicks">{clicks()}</text>
        <text testId="events">{events()}</text>
        <text testId="retained">{retained()}</text>
        <text testId="authority">{authority()}</text>
      </column>
    );
  }, root);
}
