import { createSignal, mountShellStylesheet, PopupSurface, renderApp, WindowSurface } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountShellStylesheet("reset", ":root { padding: 0; }");
    const [bodyHeight, setBodyHeight] = createSignal(120);
    const [windowWidth, setWindowWidth] = createSignal(480);
    const [windowTitle, setWindowTitle] = createSignal("Settings");
    return <column>
      <control testId="set-260" onClick={() => setBodyHeight(260)}>Set 260</control>
      <control testId="set-700" onClick={() => setBodyHeight(700)}>Set 700</control>
      <control testId="set-80" onClick={() => setBodyHeight(80)}>Set 80</control>
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
    </column>;
  }, root);
}
