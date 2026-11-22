import { Button, createSignal, isAnimating, mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeAnimationEvent, NativeNode, NativeTransitionEvent } from "@hewel/zgui";
export const hostApiVersion = 1;
const buttonStyle = {
  width: "150px",
  height: "32px",
  padding: "6px",
  background: "#374151",
  color: "#ffffff",
};
const css = `
.anim-sample, .anim-slide, .anim-pseudo { width: 200px; height: 80px; background-color: #bbf7d0; }
.anim-sample[data-run="yes"] { animation: fast-paint 80ms linear both, slow-fade 200ms linear both; }
@keyframes fast-paint { from { background-color: #bbf7d0; } to { background-color: #22c55e; } }
@keyframes slow-fade { from { opacity: 0.2; } to { opacity: 1; } }
.anim-slide { width: 160px; background-color: #bae6fd; transition: width 160ms linear; }
.anim-slide[data-wide="yes"] { width: 240px; }
.anim-slide[data-enabled="no"] { transition: none; }
.anim-pseudo { background-color: #fef3c7; }
.anim-pseudo::before { content: "P"; }
.anim-pseudo[data-run="yes"]::before { animation: pseudo-fade 120ms linear both; }
@keyframes pseudo-fade { from { opacity: 0.2; } to { opacity: 1; } }
`;
export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("animation-fixture", css);
    const [running, setRunning] = createSignal(false);
    const [wide, setWide] = createSignal(false);
    const [enabled, setEnabled] = createSignal(true);
    const [pseudo, setPseudo] = createSignal(false);
    const [ends, setEnds] = createSignal<string[]>([]);
    const [cancels, setCancels] = createSignal<string[]>([]);
    const [transitionEnd, setTransitionEnd] = createSignal("none");
    const [transitionCancel, setTransitionCancel] = createSignal("none");
    const [pseudoEnd, setPseudoEnd] = createSignal("none");
    const [queried, setQueried] = createSignal("unread");
    const [retained, setRetained] = createSignal("unread");
    let holder!: NativeNode;
    let sample!: NativeNode;
    let slide!: NativeNode;
    let generated!: NativeNode;
    let held: NativeAnimationEvent | undefined;
    let captureCount = 0;
    const ended = (event: NativeAnimationEvent) => {
      if (event.target !== sample) return;
      held = event;
      setEnds(values => [
        ...values,
        `${event.animationName}:${event.elapsedTime.toFixed(3)}:${isAnimating(sample)}:${captureCount}`,
      ]);
    };
    const transition = (event: NativeTransitionEvent) =>
      `${event.propertyName}:${event.elapsedTime.toFixed(3)}:${isAnimating(slide)}`;
    return (
      <column
        style={{
          width: "740px",
          padding: "12px",
          gap: "8px",
          background: "#ffffff",
          color: "#18181b",
          "font-size": "14px",
        }}
      >
        <row style={{ gap: "8px" }}>
          <Button testId="start" style={buttonStyle} onClick={() => setRunning(true)}>
            Start pair
          </Button>
          <Button testId="stop" style={buttonStyle} onClick={() => setRunning(false)}>
            Cancel pair
          </Button>
          <Button
            testId="clear"
            style={buttonStyle}
            onClick={() => {
              setEnds([]);
              setCancels([]);
              captureCount = 0;
            }}
          >
            Clear log
          </Button>
          <Button
            testId="read"
            style={buttonStyle}
            onClick={() =>
              setQueried(
                [holder, sample, slide, generated].map(node => String(isAnimating(node))).join(","),
              )
            }
          >
            Read state
          </Button>
        </row>
        <row style={{ gap: "8px" }}>
          <Button
            testId="transition-start"
            style={buttonStyle}
            onClick={() => {
              setEnabled(true);
              setWide(true);
            }}
          >
            Start transition
          </Button>
          <Button
            testId="transition-stop"
            style={buttonStyle}
            onClick={() => {
              setEnabled(false);
              setWide(false);
            }}
          >
            Cancel transition
          </Button>
          <Button testId="pseudo-start" style={buttonStyle} onClick={() => setPseudo(true)}>
            Start pseudo
          </Button>
          <Button
            testId="retained"
            style={buttonStyle}
            onClick={() => {
              if (!held) return;
              let expired = false;
              try {
                held.preventDefault();
              } catch (error) {
                expired = error instanceof ReferenceError;
              }
              setRetained(
                `${held.animationName}:${held.elapsedTime.toFixed(3)}:${held.pseudoElement}:${expired}`,
              );
            }}
          >
            Read saved event
          </Button>
        </row>
        <row
          ref={node => {
            holder = node;
          }}
          testId="holder"
          style={{ gap: "8px" }}
          onAnimationEndCapture={event => {
            if (event.target === sample) captureCount += 1;
          }}
        >
          <box
            ref={node => {
              sample = node;
            }}
            testId="sample"
            class="anim-sample"
            data-run={running() ? "yes" : "no"}
            onAnimationEnd={ended}
            onAnimationCancel={event =>
              setCancels(values =>
                [...values, `${event.animationName}:${isAnimating(sample)}`].sort(),
              )
            }
          />
          <box
            ref={node => {
              slide = node;
            }}
            testId="slide"
            class="anim-slide"
            data-wide={wide() ? "yes" : "no"}
            data-enabled={enabled() ? "yes" : "no"}
            onTransitionEnd={event => setTransitionEnd(transition(event))}
            onTransitionCancel={event =>
              setTransitionCancel(`${event.propertyName}:${isAnimating(slide)}`)
            }
          />
          <box
            ref={node => {
              generated = node;
            }}
            testId="pseudo"
            class="anim-pseudo"
            data-run={pseudo() ? "yes" : "no"}
            onAnimationEnd={event =>
              setPseudoEnd(
                `${event.animationName}:${event.pseudoElement}:${event.elapsedTime.toFixed(3)}:${isAnimating(generated)}`,
              )
            }
          />
        </row>
        <text testId="ends">{ends().join("|")}</text>
        <text testId="cancels">{cancels().join("|")}</text>
        <text testId="transition-end">{transitionEnd()}</text>
        <text testId="transition-cancel">{transitionCancel()}</text>
        <text testId="pseudo-end">{pseudoEnd()}</text>
        <text testId="queried">{queried()}</text>
        <text testId="retained-value">{retained()}</text>
      </column>
    );
  }, root);
}
