import {
  Portal,
  Show,
  createEffect,
  createSignal,
  onSettled,
  registerDismissLayer,
  renderApp,
  trapFocus,
} from "@hewel/zgui";
import type { NativeChild, NativeDismissReason, NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";

export const hostApiVersion = 1;

function Surface(props: {
  name: string;
  x: number;
  exclude: NativeNode;
  locked?: boolean;
  leaving?: boolean;
  dismiss: (reason: NativeDismissReason) => void;
  unhandled: () => void;
  children?: NativeChild;
}) {
  let node!: NativeNode;
  onSettled(() => {
    const options = () => ({
      layer: "modal" as const,
      exclude: props.exclude,
      dismissOnOutsidePress: !props.locked,
      dismissOnEscape: !props.locked,
      leaving: props.leaving ?? false,
    });
    const lease = registerDismissLayer(node, props.dismiss, options());
    createEffect(options, next => {
      lease.update(next);
    });
    trapFocus(node);
  });
  return (
    <column
      ref={value => {
        node = value;
      }}
      testId={props.name}
      onKeyDown={event => {
        if (event.key === "Escape") props.unhandled();
      }}
      style={`position: absolute; left: ${props.x}px; top: 80px; width: 220px; padding: 12px; gap: 8px; background-color: ${props.name === "lower" ? "#e0f2fe" : "#fef3c7"}; color: #18181b`}
    >
      <text>{props.name}</text>
      <text testId={`${props.name}-state`}>{props.leaving ? "leaving" : "open"}</text>
      {props.children}
    </column>
  );
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [source, setSource] = createSignal(true);
    const [lower, setLower] = createSignal(false);
    const [upper, setUpper] = createSignal(false);
    const [locked, setLocked] = createSignal(false);
    const [leaving, setLeaving] = createSignal(false);
    const [last, setLast] = createSignal("none");
    const [dismisses, setDismisses] = createSignal(0);
    const [presses, setPresses] = createSignal(0);
    const [clicks, setClicks] = createSignal(0);
    const [unhandled, setUnhandled] = createSignal(0);
    let launch!: NativeNode;
    let trigger!: NativeNode;
    const report = (name: string, reason: NativeDismissReason) => {
      setLast(`${name}:${reason}`);
      setDismisses(value => value + 1);
    };
    const leaked = () => setUnhandled(value => value + 1);
    return (
      <ThemeProvider scheme="light">
        <column
          onKeyDown={event => {
            if (event.key === "Escape") leaked();
          }}
          style={{
            width: "740px",
            padding: "12px",
            gap: "8px",
            "background-color": "var(--zui-color-background)",
            color: "var(--zui-color-foreground)",
          }}
        >
          <column style={{ width: "200px", gap: "8px" }}>
            <Button
              ref={node => {
                launch = node;
              }}
              testId="launch"
              onClick={() => {
                setUpper(false);
                setLocked(false);
                setLeaving(false);
                setLower(value => !value);
              }}
            >
              Toggle lower
            </Button>
            <Button
              testId="outside"
              onPointerDown={() => setPresses(value => value + 1)}
              onClick={() => setClicks(value => value + 1)}
            >
              Outside action
            </Button>
            <Button testId="source-toggle" onClick={() => setSource(value => !value)}>
              Toggle source
            </Button>
            <text testId="last">{last()}</text>
            <text testId="dismisses">{dismisses()}</text>
            <text testId="presses">{presses()}</text>
            <text testId="clicks">{clicks()}</text>
            <text testId="unhandled">{unhandled()}</text>
          </column>
          <Show when={source()}>
            <box testId="source" style="width: 180px; height: 50px; overflow: hidden">
              <text>Portal source</text>
              <Show when={lower()}>
                <Portal layer="modal">
                  <Surface
                    name="lower"
                    x={260}
                    exclude={launch}
                    unhandled={leaked}
                    dismiss={reason => {
                      report("lower", reason);
                      setLower(false);
                      setUpper(false);
                    }}
                  >
                    <Button testId="lower-first">Lower first</Button>
                    <Button
                      ref={node => {
                        trigger = node;
                      }}
                      testId="upper-open"
                      onClick={() => {
                        setLocked(false);
                        setLeaving(false);
                        setUpper(value => !value);
                      }}
                    >
                      Toggle upper
                    </Button>
                    <Show when={upper()}>
                      <Portal layer="modal">
                        <Surface
                          name="upper"
                          x={510}
                          exclude={trigger}
                          locked={locked()}
                          leaving={leaving()}
                          unhandled={leaked}
                          dismiss={reason => {
                            report("upper", reason);
                            if (reason === "escape") setLeaving(true);
                            else setUpper(false);
                          }}
                        >
                          <Button testId="upper-first">Upper first</Button>
                          <Button testId="upper-lock" onClick={() => setLocked(value => !value)}>
                            <text>{locked() ? "Unlock dismissal" : "Lock dismissal"}</text>
                          </Button>
                        </Surface>
                      </Portal>
                    </Show>
                  </Surface>
                </Portal>
              </Show>
            </box>
          </Show>
        </column>
      </ThemeProvider>
    );
  }, root);
}
