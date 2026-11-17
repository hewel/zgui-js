import { Button, Show, createSignal, observeGeometry, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode, NativeSize, NativeViewport } from "@hewel/zgui";
export const hostApiVersion = 1;
const buttonStyle = { width: "120px", height: "32px", padding: "6px", background: "#374151", color: "#ffffff" };
const describe = (size: NativeSize | null) => size ? `${size.width}x${size.height}` : "pending";
function Measured(props: { width: number }) {
  const [anchor, setAnchor] = createSignal<NativeSize | null>(null);
  const [follower, setFollower] = createSignal<NativeSize | null>(null);
  const [zero, setZero] = createSignal<NativeSize | null>(null);
  const [late, setLate] = createSignal<NativeSize | null>(null);
  const [viewport, setViewport] = createSignal<NativeViewport | null>(null);
  const viewportReport = () => { const v = viewport(); return v ? `viewport:${v.width}x${v.height}@${v.scale}` : "viewport:pending"; };
  const [followerWidth, setFollowerWidth] = createSignal(0);
  const [anchorCalls, setAnchorCalls] = createSignal(0);
  let anchorNode!: NativeNode;
  let stopAnchor = () => {};
  let stopLate = () => {};
  onCleanup(() => stopLate());
  const report = () => "geometry:" + JSON.stringify({ anchor: describe(anchor()), follower: describe(follower()), zero: describe(zero()), late: describe(late()), anchorCalls: anchorCalls() });
  return <column testId="geometry-pair" style={{ gap: "8px" }} ref={node => { observeGeometry(node, "viewport", setViewport); }}>
    <row style={{ gap: "12px", height: "40px" }}>
      <box testId="anchor" style={{ width: `${props.width}px`, height: "40px", background: "#0f766e" }} ref={node => {
        anchorNode = node;
        stopAnchor = observeGeometry(node, "borderSize", size => { setAnchor(size); setAnchorCalls(n => n + 1); setFollowerWidth(size.width); });
      }} />
      <box testId="follower" style={{ width: `${followerWidth()}px`, height: "40px", background: "#2563eb" }} ref={node => { observeGeometry(node, "borderSize", setFollower); }} />
      <box testId="zero" style={{ width: "0px", height: "0px" }} ref={node => { observeGeometry(node, "borderSize", setZero); }} />
    </row>
    <text testId="geometry-report">{report()}</text>
    <text testId="viewport-report">{viewportReport()}</text>
    <row style={{ gap: "8px" }}>
      <Button testId="late" style={buttonStyle} onClick={() => { stopLate(); stopLate = observeGeometry(anchorNode, "borderSize", setLate); }}>Subscribe</Button>
      <Button testId="cancel" style={buttonStyle} onClick={() => { stopAnchor(); stopAnchor(); }}>Stop anchor</Button>
    </row>
  </column>;
}
export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const [width, setWidth] = createSignal(100);
    const [shown, setShown] = createSignal(true);
    return <column style={{ width: "640px", padding: "12px", gap: "12px", color: "#e2e8f0", "font-size": "14px" }}>
      <row style={{ gap: "8px" }}>
        <Button testId="resize" style={buttonStyle} onClick={() => setWidth(w => w + 60)}>Resize</Button>
        <Button testId="toggle" style={buttonStyle} onClick={() => setShown(v => !v)}>Toggle</Button>
      </row>
      <Show when={shown()}><Measured width={width()} /></Show>
    </column>;
  }, root);
}
