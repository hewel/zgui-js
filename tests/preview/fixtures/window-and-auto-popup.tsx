import { createSignal, mountShellStylesheet, PopupSurface, renderApp, Show, WindowSurface } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountShellStylesheet("reset", ":root { padding: 0; }");
    const [bodyHeight, setBodyHeight] = createSignal(120);
    const [windowWidth, setWindowWidth] = createSignal(480);
    const [windowTitle, setWindowTitle] = createSignal("Settings");
    const [showShaped, setShowShaped] = createSignal(false);
    const [shapedHits, setShapedHits] = createSignal(0);
    return <column>
      <control testId="set-260" onClick={() => setBodyHeight(260)}>Set 260</control>
      <control testId="set-700" onClick={() => setBodyHeight(700)}>Set 700</control>
      <control testId="set-80" onClick={() => setBodyHeight(80)}>Set 80</control>
      <control testId="open-shaped" onClick={() => setShowShaped(true)}>Open shaped popup</control>
      <control testId="resize-win" onClick={() => {
        setWindowWidth(560);
        setWindowTitle("Settings — Resized");
      }}>Resize window</control>
      <WindowSurface
        name="settings"
        title={windowTitle()}
        appId="z-shell-settings"
        width={windowWidth()}
        height={320}
        minWidth={320}
        minHeight={240}
        decorations="full"
        transparent={false}
      >
        <column style={{ width: "100%", height: "100%", backgroundColor: "#1b202b", padding: "16px" }}>
          <text testId="settings-title">{windowTitle()}</text>
        </column>
      </WindowSurface>
      <PopupSurface
        name="auto-menu"
        parent="settings"
        anchorRect={{ x: 16, y: 16, width: 80, height: 28 }}
        width={240}
        height="auto"
        minHeight={48}
        maxHeight={400}
      >
        <scroll testId="auto-scroll" style={{ maxHeight: "100vh", backgroundColor: "#243044" }}>
          <box testId="auto-body" style={{ width: "100%", height: `${bodyHeight()}px`, backgroundColor: "#3b82f6" }}>
            <text testId="auto-label">{`h=${bodyHeight()}`}</text>
          </box>
        </scroll>
      </PopupSurface>
      <Show when={showShaped()}>
        <PopupSurface
          name="shaped-menu"
          parent="settings"
          anchorRect={{ x: 120, y: 16, width: 80, height: 28 }}
          width={200}
          height={160}
          inputRegion={[{ x: 0, y: 0, width: 120, height: 60 }]}
        >
          <column style={{ width: "200px", height: "160px" }}>
            <control testId="inside-hit" style={{ width: "100px", height: "40px", backgroundColor: "#22c55e" }} onClick={() => setShapedHits(n => n + 1)}>
              <text testId="shaped-hits">{`hits=${shapedHits()}`}</text>
            </control>
            <control testId="outside-hit" style={{ width: "100px", height: "40px", marginTop: "40px", backgroundColor: "#ef4444" }} onClick={() => setShapedHits(n => n + 10)}>
              <text>outside</text>
            </control>
          </column>
        </PopupSurface>
      </Show>
    </column>;
  }, root);
}
