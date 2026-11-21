import { Show, WindowSurface } from "@hewel/zgui";
import { Badge, Button, Separator, Switch, ThemeProvider } from "@hewel/zgui/components";
import { useStudio } from "../studio-context.js";

export function AboutPrefsWindow() {
  const studio = useStudio();

  return (
    <Show when={studio.prefsWindowOpen()}>
      <WindowSurface
        name="about-prefs"
        title="Markdown Studio — About & Quick Preferences"
        width={420}
        height={320}
        minWidth={340}
        minHeight={260}
        onClose={() => studio.setPrefsWindowOpen(false)}
      >
        <ThemeProvider
          scheme={studio.scheme()}
          light={{ preset: studio.preset() }}
          dark={{ preset: studio.preset() }}
        >
          <column
            testId="aux-about-root"
            style={{
              width: "100%",
              height: "100%",
              padding: "16px",
              gap: "12px",
              "background-color": "var(--zui-color-background)",
              color: "var(--zui-color-foreground)",
              "box-sizing": "border-box",
            }}
          >
            <row style={{ "justify-content": "space-between", "align-items": "center" }}>
              <text style={{ "font-size": "18px", "font-weight": "700" }}>zgui Markdown Studio</text>
              <Badge variant="secondary">Wave 1</Badge>
            </row>
            <text class="muted">
              Local-first native Markdown workbench built on Rust + wgpu + Stylo + QuickJS-NG + Solid 2.0.
            </text>
            <Separator />
            <row style={{ "justify-content": "space-between", "align-items": "center" }}>
              <text testId="aux-theme-readout">{`Active Theme: ${studio.preset()} / ${studio.scheme()}`}</text>
              <Switch
                testId="aux-dark-switch"
                checked={studio.scheme() === "dark"}
                onCheckedChange={on => studio.setScheme(on ? "dark" : "light")}
                ariaLabel="Toggle theme from auxiliary window"
              />
            </row>
            <row style={{ "justify-content": "space-between", "align-items": "center" }}>
              <text testId="aux-doc-readout">{`Active Doc: ${studio.activeDoc().title}`}</text>
              <Badge variant="outline">{`FTS5: ${studio.indexedCount()}`}</Badge>
            </row>
            <row style={{ "justify-content": "flex-end", gap: "8px", "margin-top": "auto" }}>
              <Button testId="aux-close-btn" size="sm" onClick={() => studio.setPrefsWindowOpen(false)}>
                Close
              </Button>
            </row>
          </column>
        </ThemeProvider>
      </WindowSurface>
    </Show>
  );
}
