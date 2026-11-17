import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, DropdownMenu, DropdownMenuContent, DropdownMenuTrigger, MenuCheckboxItem, MenuItem, MenuLabel,
  MenuRadioGroup, MenuRadioItem, MenuSeparator, ThemeProvider } from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dark, setDark] = createSignal(false), [shown, setShown] = createSignal(true);
    const [opened, setOpened] = createSignal(false), [upload, setUpload] = createSignal(false), [shortcuts, setShortcuts] = createSignal(true);
    const [sort, setSort] = createSignal("name"), [radioChanges, setRadioChanges] = createSignal(0);
    const [selections, setSelections] = createSignal(0), [last, setLast] = createSignal("none");
    const bumps = new Map<string, () => void>();
    function Counter(props: { id: string }) {
      const [count, setCount] = createSignal(0);
      const bump = () => setCount(value => value + 1);
      bumps.set(props.id, bump);
      onCleanup(() => { if (bumps.get(props.id) === bump) bumps.delete(props.id); });
      return <text testId={props.id + "-count"}>{count()}</text>;
    }
    const select = (value: string) => { setLast(value); setSelections(count => count + 1); };
    return <ThemeProvider scheme={dark() ? "dark" : "light"}>
      <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
        <box style="position: fixed; inset: 0; overflow-y: auto">
          <box testId="page" style="height: 1400px; flex-shrink: 0" />
        </box>
        <Show when={shown()}><DropdownMenu onOpenChange={setOpened}>
          <DropdownMenuTrigger testId="trigger" variant="outline" style="position: fixed; left: 32px; top: 24px; width: 180px; height: 36px">Actions</DropdownMenuTrigger>
          <DropdownMenuContent testId="menu" ariaLabel="Actions" style="width: 260px">
            <MenuLabel testId="label">File actions</MenuLabel>
            <MenuItem testId="print" ariaLabel="Print" closeOnSelect={false} shortcut={shortcuts() ? "Ctrl+P" : null}
              onSelect={() => { select("print"); bumps.get("print")?.(); }}>
              <text testId="print-body">Print <Counter id="print" /></text>
            </MenuItem>
            <MenuItem testId="disabled" disabled onSelect={() => select("disabled")}>Paused</MenuItem>
            <MenuItem testId="properties" closeOnSelect={false} onSelect={() => select("properties")}>Properties</MenuItem>
            <MenuItem testId="save" onSelect={() => select("save")}>Save</MenuItem>
            <MenuItem testId="send" closeOnSelect={false} onSelect={() => select("send")}><text>{upload() ? "Upload" : "Send now"}</text></MenuItem>
            <MenuSeparator />
            <MenuCheckboxItem testId="upload" checked={upload()} closeOnSelect={false}
              onCheckedChange={value => { setUpload(value); bumps.get("check")?.(); }}>
              <text>Use upload action <Counter id="check" /></text>
            </MenuCheckboxItem>
            <MenuCheckboxItem testId="shortcuts" checked={shortcuts()} closeOnSelect={false} onCheckedChange={setShortcuts}>Show shortcuts</MenuCheckboxItem>
            <MenuRadioGroup label="Sort order" value={sort()} onValueChange={value => { setSort(value); setRadioChanges(count => count + 1); }}>
              <MenuLabel inset>Sort order</MenuLabel>
              <MenuRadioItem testId="name" value="name" closeOnSelect={false}>Name</MenuRadioItem>
              <MenuRadioItem testId="date" value="date" closeOnSelect={false}>Date</MenuRadioItem>
            </MenuRadioGroup>
            <MenuSeparator />
            <MenuItem testId="theme" closeOnSelect={false} onSelect={() => setDark(value => !value)}>Toggle theme</MenuItem>
            <MenuItem testId="delete" destructive onSelect={() => select("delete")}>Delete draft</MenuItem>
          </DropdownMenuContent>
        </DropdownMenu></Show>
        <Button testId="toggle-source" variant="outline" style="position: fixed; left: 360px; top: 24px" onClick={() => setShown(value => !value)}>Toggle source</Button>
        <Button testId="outside" variant="outline" style="position: fixed; left: 560px; top: 24px">Outside</Button>
        <column style="position: fixed; left: 360px; top: 90px; width: 400px; gap: 10px; font-size: 14px">
          <text>Menu state / selection count / last command</text>
          <text testId="open">{opened() ? "open" : "closed"}</text>
          <text testId="selections">{selections()}</text><text testId="last">{last()}</text>
          <text>Upload / shortcuts / sort / radio changes</text>
          <text testId="upload-state">{String(upload())}</text><text testId="shortcut-state">{String(shortcuts())}</text>
          <text testId="sort">{sort()}</text><text testId="radio-changes">{radioChanges()}</text>
        </column>
      </box>
    </ThemeProvider>;
  }, root);
}
