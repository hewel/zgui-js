import { Show, createSignal, onCleanup, renderApp, untrack } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  MenuCheckboxItem,
  MenuItem,
  MenuLabel,
  MenuSub,
  MenuSubContent,
  MenuSubTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true),
      [dark, setDark] = createSignal(false),
      [flipped, setFlipped] = createSignal(false);
    const [rootRequest, setRootRequest] = createSignal(false),
      [subRequest, setSubRequest] = createSignal(false);
    const [subChanges, setSubChanges] = createSignal(0),
      [flag, setFlag] = createSignal(false);
    const [selected, setSelected] = createSignal("none"),
      [selections, setSelections] = createSignal(0);
    const bumpers: Record<string, () => void> = {};
    function Counter(props: { id: string }) {
      const id = untrack(() => props.id);
      const [count, setCount] = createSignal(0);
      bumpers[id] = () => setCount(v => v + 1);
      onCleanup(() => {
        delete bumpers[id];
      });
      return <text testId={props.id}>{count()}</text>;
    }
    const select = (value: string) => {
      setSelected(value);
      setSelections(v => v + 1);
    };
    return (
      <ThemeProvider scheme={dark() ? "dark" : "light"}>
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <Show when={shown()}>
            <DropdownMenu onOpenChange={setRootRequest}>
              <DropdownMenuTrigger
                testId="root-trigger"
                style={`position: fixed; left: ${flipped() ? 580 : 32}px; top: 24px; width: 180px`}
              >
                Files
              </DropdownMenuTrigger>
              <DropdownMenuContent testId="menu" ariaLabel="Files" style="width: 260px">
                <MenuLabel>Actions</MenuLabel>
                <MenuItem testId="before" closeOnSelect={false}>
                  Before
                </MenuItem>
                <MenuSub
                  onOpenChange={value => {
                    setSubRequest(value);
                    setSubChanges(v => v + 1);
                  }}
                >
                  <MenuSubTrigger testId="branch">
                    <text>Export</text>
                    <Counter id="trigger-count" />
                  </MenuSubTrigger>
                  <MenuSubContent testId="submenu" ariaLabel="Export" style="width: 240px">
                    <MenuItem
                      testId="keep"
                      closeOnSelect={false}
                      onSelect={() => bumpers["sub-count"]?.()}
                    >
                      <text>Keep</text>
                      <Counter id="sub-count" />
                    </MenuItem>
                    <MenuCheckboxItem
                      testId="flag"
                      checked={flag()}
                      onCheckedChange={setFlag}
                      closeOnSelect={false}
                    >
                      Pin format
                    </MenuCheckboxItem>
                    <MenuSub>
                      <MenuSubTrigger testId="deep-trigger">More</MenuSubTrigger>
                      <MenuSubContent testId="deep" ariaLabel="More formats" style="width: 200px">
                        <MenuItem testId="deep-disabled" disabled>
                          Unavailable
                        </MenuItem>
                        <MenuItem testId="leaf" onSelect={() => select("PDF")}>
                          PDF
                        </MenuItem>
                        <MenuItem testId="deep-keep" closeOnSelect={false}>
                          Stay
                        </MenuItem>
                      </MenuSubContent>
                    </MenuSub>
                    <MenuItem
                      testId="theme"
                      closeOnSelect={false}
                      onSelect={() => setDark(v => !v)}
                    >
                      Toggle theme
                    </MenuItem>
                    <MenuItem testId="csv" onSelect={() => select("CSV")}>
                      CSV
                    </MenuItem>
                  </MenuSubContent>
                </MenuSub>
                <MenuItem testId="after" closeOnSelect={false}>
                  After
                </MenuItem>
                <MenuSub>
                  <MenuSubTrigger testId="disabled-branch" disabled>
                    Disabled export
                  </MenuSubTrigger>
                  <MenuSubContent testId="disabled-submenu">
                    <MenuItem>Never</MenuItem>
                  </MenuSubContent>
                </MenuSub>
              </DropdownMenuContent>
            </DropdownMenu>
          </Show>
          <Button
            testId="toggle-source"
            variant="outline"
            style="position: fixed; left: 240px; top: 24px"
            onClick={() => setShown(v => !v)}
          >
            Toggle source
          </Button>
          <Button
            testId="edge"
            variant="outline"
            style="position: fixed; left: 400px; top: 24px"
            onClick={() => setFlipped(v => !v)}
          >
            Switch edge
          </Button>
          <Button
            testId="outside"
            variant="outline"
            style="position: fixed; left: 32px; top: 340px"
          >
            Outside
          </Button>
          <column style="position: fixed; left: 32px; top: 410px; width: 720px; gap: 5px; font-size: 14px">
            <text>
              Last root request / last branch request / branch changes / pinned / selection
            </text>
            <row style="gap: 20px">
              <text testId="root-state">{rootRequest() ? "open" : "closed"}</text>
              <text testId="sub-state">{subRequest() ? "open" : "closed"}</text>
              <text testId="sub-changes">{subChanges()}</text>
              <text testId="pinned">{flag() ? "true" : "false"}</text>
              <text testId="selected">{selected()}</text>
              <text testId="selections">{selections()}</text>
            </row>
          </column>
        </box>
      </ThemeProvider>
    );
  }, root);
}
