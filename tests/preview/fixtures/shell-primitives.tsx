import * as stylex from "@stylexjs/stylex";
import { createSignal, createSpring, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

const styles = stylex.create({
  glass: { backdropFilter: "grayscale(1)" },
  lettering: { fontVariationSettings: '"wght" 650', textShadow: "24px 0 #0000ff" },
});

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [target, setTarget] = createSignal(0);
    const width = createSpring(target);
    const [log, setLog] = createSignal("");
    return <column style="position: fixed; inset: 0; background: white; color: black">
      <box style="position: fixed; left: 16px; top: 16px; width: 144px; height: 48px; background: red" />
      <box {...stylex.props(styles.glass)} style="position: fixed; left: 88px; top: 16px; width: 72px; height: 48px" />
      <box style={{ position: "fixed", left: "184px", top: "16px", width: "48px", height: "48px",
        background: "black", boxShadow: "12px 0 #00ff00" }} />
      <text {...stylex.props(styles.lettering)} style="position: fixed; left: 280px; top: 16px; font-size: 30px">Hi</text>
      <scroll testId="wheel-scroll" style="position: fixed; left: 16px; top: 100px; width: 120px; height: 100px; overflow: auto"
        onWheelCapture={event => {
          const { x, y, unit } = event.delta;
          setLog(`${event.phase}:${event.pointerKind}:${unit}:${event.scrollPhase}:${x},${y}:${event.position.x},${event.position.y}`);
          if (x !== 0) {
            event.preventDefault();
            event.stopPropagation();
          }
        }} onWheel={event => setLog(previous => `${previous}|${event.phase}`)}>
        <column style="height: 400px">
          <box testId="wheel-marker" style="height: 56px; background: blue"
            onWheel={event => setLog(previous => `${previous}|${event.phase}`)} />
        </column>
      </scroll>
      <text testId="wheel-log" style="position: fixed; left: 16px; top: 215px">{log()}</text>
      <control testId="start" style="position: fixed; left: 184px; top: 100px; width: 96px; height: 32px"
        onClick={() => setTarget(100)}>Start spring</control>
      <box testId="spring" style={{ position: "fixed", left: "184px", top: "156px", height: "24px",
        width: `${40 + width()}px`, background: "blue" }} />
    </column>;
  }, root);
}
