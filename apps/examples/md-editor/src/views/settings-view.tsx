import { For, Show } from "@hewel/zgui";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Select,
  Slider,
  Switch,
} from "@hewel/zgui/components";
import type { SelectOption, ThemePreset } from "@hewel/zgui/components";
import { useStudio } from "../studio-context.js";

const PRESET_OPTIONS: readonly SelectOption<ThemePreset>[] = [
  { value: "base", label: "Base", description: "Balanced neutral workbench" },
  { value: "ocean", label: "Ocean", description: "Cool slate & cyan accents" },
  { value: "midnight", label: "Midnight", description: "Deep blue high-contrast" },
  { value: "ember", label: "Ember", description: "Warm amber & charcoal" },
  { value: "terminal", label: "Terminal", description: "Crisp monochrome hacker look" },
];

export function SettingsView() {
  const studio = useStudio();

  return (
    <scroll class="mde-page-scroll" testId="view-settings">
      <column class="mde-page-inner">
        <row style={{ "justify-content": "space-between", "align-items": "center" }}>
          <column style={{ gap: "4px" }}>
            <text style={{ "font-size": "20px", "font-weight": "700" }}>Studio Preferences</text>
            <text class="muted">All settings are persisted to app-scoped SQLite KV (`studio/prefs`).</text>
          </column>
          <Button testId="settings-reset" variant="outline" size="sm" onClick={() => void studio.resetPrefs()}>
            Reset Defaults
          </Button>
        </row>

        <Card>
          <CardHeader>
            <CardTitle>Appearance & Typography</CardTitle>
            <CardDescription>Customize ThemeProvider scheme, preset, and live preview scale.</CardDescription>
          </CardHeader>
          <CardContent>
            <column style={{ gap: "14px" }}>
              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <column style={{ gap: "2px" }}>
                  <text style={{ "font-weight": "600" }}>Theme Preset</text>
                  <text class="muted">Applied across main window, overlays, and auxiliary windows.</text>
                </column>
                <Select
                  testId="settings-preset-select"
                  listTestId="settings-preset-list"
                  options={PRESET_OPTIONS}
                  value={studio.preset()}
                  onValueChange={studio.setPreset}
                  label="Theme preset"
                />
              </row>

              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <column style={{ gap: "2px" }}>
                  <text style={{ "font-weight": "600" }}>Dark Mode</text>
                  <text class="muted">Toggle between dark and light color schemes.</text>
                </column>
                <Switch
                  testId="settings-dark-switch"
                  checked={studio.scheme() === "dark"}
                  onCheckedChange={on => studio.setScheme(on ? "dark" : "light")}
                  ariaLabel="Toggle dark scheme"
                />
              </row>

              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <column style={{ gap: "2px" }}>
                  <text style={{ "font-weight": "600" }}>{`Preview Font Size (${studio.fontSize()}px)`}</text>
                  <text class="muted">Persisted across sessions via SQLite KV.</text>
                </column>
                <box style={{ width: "160px" }}>
                  <Slider
                    testId="settings-font-slider"
                    value={studio.fontSize()}
                    onValueChange={studio.setFontSize}
                    min={13}
                    max={20}
                    step={1}
                    label="Preview font size"
                  />
                </box>
              </row>

              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <column style={{ gap: "2px" }}>
                  <text style={{ "font-weight": "600" }}>Unsaved Navigation Guard</text>
                  <text class="muted">Confirm via useBeforeLeave before leaving the editor with unsaved edits.</text>
                </column>
                <Switch
                  testId="settings-guard-switch"
                  checked={studio.guardNavigation()}
                  onCheckedChange={studio.setGuardNavigation}
                  ariaLabel="Toggle unsaved navigation guard"
                />
              </row>
            </column>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <row style={{ "justify-content": "space-between", "align-items": "center", width: "100%" }}>
              <column style={{ gap: "2px" }}>
                <CardTitle>Persisted Workspace Bookmarks</CardTitle>
                <CardDescription>Per-app bookmarks granted via XDG FileChooser Portal.</CardDescription>
              </column>
              <Button size="xs" variant="outline" onClick={() => void studio.refreshBookmarks()}>
                Refresh
              </Button>
            </row>
          </CardHeader>
          <CardContent>
            <Show
              when={studio.bookmarks().length > 0}
              fallback={<text class="muted" testId="bookmarks-empty">No persisted workspace bookmarks yet.</text>}
            >
              <column style={{ gap: "8px" }}>
                <For each={studio.bookmarks()}>
                  {(bm, idx) => (
                    <row
                      testId={`bookmark-row-${idx()}`}
                      style={{
                        "justify-content": "space-between",
                        "align-items": "center",
                        padding: "8px 10px",
                        border: "1px solid var(--zui-color-border)",
                        "border-radius": "var(--zui-radius-md)",
                      }}
                    >
                      <row style={{ gap: "8px", "align-items": "center" }}>
                        <Badge variant="secondary">{bm.kind}</Badge>
                        <text style={{ "font-weight": "600" }}>{bm.label}</text>
                        <Badge variant="outline">{bm.mode}</Badge>
                      </row>
                      <row style={{ gap: "6px" }}>
                        <Button
                          testId={`bookmark-restore-${idx()}`}
                          size="xs"
                          variant="outline"
                          onClick={() => void studio.restoreBookmarkByToken(bm.token)}
                        >
                          Restore
                        </Button>
                        <Button
                          testId={`bookmark-revoke-${idx()}`}
                          size="xs"
                          variant="destructive"
                          onClick={() => void studio.revokeBookmarkByToken(bm.token)}
                        >
                          Revoke
                        </Button>
                      </row>
                    </row>
                  )}
                </For>
              </column>
            </Show>
          </CardContent>
        </Card>
      </column>
    </scroll>
  );
}
