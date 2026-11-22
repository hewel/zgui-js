import { For, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Input,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme } from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(() => <Panel />, root);
}
function Profile() {
  const [count, setCount] = createSignal(0);
  return (
    <column class="tabs-panel">
      <text>Display name</text>
      <Input testId="profile-field" label="Display name" defaultValue="Preview" />
      <Button
        testId="profile-count"
        size="sm"
        variant="outline"
        onClick={() => setCount(value => value + 1)}
      >
        Count: {count()}
      </Button>
      <text class="tabs-hint">Leaving this panel resets its local state.</text>
    </column>
  );
}
function Notes() {
  return (
    <column class="tabs-panel">
      <text>Draft</text>
      <Input testId="draft-field" label="Draft" defaultValue="Draft" />
      <text class="tabs-hint">This draft stays mounted while its panel is hidden.</text>
    </column>
  );
}
function Panel() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [active, setActive] = createSignal("profile");
  const [manual, setManual] = createSignal("overview");
  const [disabled, setDisabled] = createSignal(false);
  const [notesShown, setNotesShown] = createSignal(true);
  const [order, setOrder] = createSignal(["profile", "locked", "notes", "history"]);
  const names: Record<string, string> = {
    profile: "Profile",
    locked: "Locked",
    notes: "Notes",
    history: "History",
  };
  return (
    <ThemeProvider scheme={scheme()}>
      <scroll class="tabs-scroll">
        <column class="tabs-page">
          <row class="tabs-header">
            <text class="tabs-heading">Tabs</text>
            <Button
              testId="tabs-theme"
              size="sm"
              variant="outline"
              onClick={() => setScheme(value => (value === "light" ? "dark" : "light"))}
            >
              Theme
            </Button>
          </row>
          <row class="tabs-columns">
            <column class="tabs-column">
              <text class="tabs-title">Automatic activation</text>
              <row class="tabs-actions">
                <Button
                  testId="swap-tabs"
                  size="sm"
                  variant="outline"
                  onClick={() => setOrder(items => [items[2], items[1], items[0], items[3]])}
                >
                  Swap
                </Button>
                <Button
                  testId="disable-tabs"
                  size="sm"
                  variant="outline"
                  onClick={() => setDisabled(value => !value)}
                >
                  {disabled() ? "Enable" : "Disable"}
                </Button>
                <Button
                  testId="show-notes"
                  size="sm"
                  variant="outline"
                  onClick={() => setNotesShown(value => !value)}
                >
                  {notesShown() ? "Hide notes" : "Show notes"}
                </Button>
                <Button testId="before-auto" size="sm" variant="outline">
                  Focus tabs
                </Button>
              </row>
              <Tabs
                testId="automatic"
                defaultValue="profile"
                onValueChange={setActive}
                disabled={disabled()}
                label="Automatic tabs"
              >
                <TabsList testId="auto-list" label="Automatic tabs">
                  <For each={order().filter(value => notesShown() || value !== "notes")}>
                    {value => (
                      <TabsTrigger
                        testId={`auto-${value}`}
                        value={value}
                        disabled={value === "locked"}
                      >
                        {names[value]}
                      </TabsTrigger>
                    )}
                  </For>
                </TabsList>
                <TabsContent testId="auto-panel-profile" value="profile">
                  <Profile />
                </TabsContent>
                <TabsContent testId="auto-panel-locked" value="locked">
                  <text>Unavailable.</text>
                </TabsContent>
                <TabsContent testId="auto-panel-notes" value="notes" keepMounted>
                  <Notes />
                </TabsContent>
                <TabsContent testId="auto-panel-history" value="history">
                  <text testId="history-body">History is built when selected.</text>
                </TabsContent>
              </Tabs>
              <text testId="auto-value">Selected: {active()}</text>
              <text class="tabs-hint">Arrow keys select a tab. Disabled tabs are skipped.</text>
            </column>
            <column class="tabs-column">
              <text class="tabs-title">Manual activation</text>
              <Button testId="before-manual" size="sm" variant="outline">
                Focus manual tabs
              </Button>
              <Tabs
                testId="manual"
                value={manual()}
                onValueChange={setManual}
                activation="manual"
                orientation="vertical"
                label="Manual tabs"
              >
                <TabsList testId="manual-list" label="Manual tabs" variant="line">
                  <TabsTrigger testId="manual-overview" value="overview">
                    Overview
                  </TabsTrigger>
                  <TabsTrigger testId="manual-details" value="details">
                    Details
                  </TabsTrigger>
                  <TabsTrigger testId="manual-locked" value="locked" disabled>
                    Locked
                  </TabsTrigger>
                </TabsList>
                <TabsContent testId="manual-panel-overview" value="overview">
                  <column style={{ gap: "12px" }}>
                    <text testId="overview-body">
                      Move focus first. Press Enter or Space to open a panel.
                    </text>
                    <text class="tabs-hint">Horizontal tabs inside vertical tabs</text>
                    <Tabs
                      testId="nested"
                      defaultValue="one"
                      orientation="horizontal"
                      label="Nested pages"
                    >
                      <TabsList testId="nested-list" variant="default" label="Nested pages">
                        <box>
                          <TabsTrigger testId="nested-one" value="one">
                            One
                          </TabsTrigger>
                        </box>
                        <box>
                          <TabsTrigger testId="nested-two" value="two">
                            Two
                          </TabsTrigger>
                        </box>
                      </TabsList>
                      <TabsContent value="one">
                        <text>First nested panel.</text>
                      </TabsContent>
                      <TabsContent value="two">
                        <text>Second nested panel.</text>
                      </TabsContent>
                    </Tabs>
                  </column>
                </TabsContent>
                <TabsContent testId="manual-panel-details" value="details">
                  <text testId="details-body">Selection changes only after activation.</text>
                </TabsContent>
                <TabsContent testId="manual-panel-locked" value="locked">
                  <text>Unavailable.</text>
                </TabsContent>
              </Tabs>
              <text testId="manual-value">Selected: {manual()}</text>
            </column>
          </row>
        </column>
      </scroll>
    </ThemeProvider>
  );
}
