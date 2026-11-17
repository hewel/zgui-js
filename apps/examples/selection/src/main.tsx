import { For, Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Label, RadioGroup, RadioGroupItem, Slider, ThemeProvider, ToggleGroup, ToggleGroupItem } from "@hewel/zgui/components";
import type { ColorScheme } from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void { return renderApp(() => <Panel />, root); }
function Option(props: { id: string; value: string; label: string; disabled?: boolean }) {
  let node: NativeNode | undefined;
  return <row class="selection-row">
    <RadioGroupItem testId={props.id} ref={value => { node = value; }} value={props.value} label={props.label} disabled={props.disabled} />
    <Label testId={`${props.id}-label`} control={() => node} disabled={props.disabled}>{props.label}</Label>
  </row>;
}
function Panel() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [quality, setQuality] = createSignal("balanced");
  const [alignment, setAlignment] = createSignal<readonly string[]>(["left"]);
  const [formatting, setFormatting] = createSignal<readonly string[]>([]);
  const [order, setOrder] = createSignal(["bold", "italic", "underline"]);
  const [italicShown, setItalicShown] = createSignal(true);
  const [volume, setVolume] = createSignal(25);
  const [disabled, setDisabled] = createSignal(false);
  const [shown, setShown] = createSignal(true);
  const [pointer, setPointer] = createSignal("none");
  const [moves, setMoves] = createSignal(0);
  const names: Record<string, string> = { bold: "Bold", italic: "Italic", underline: "Underline" };
  return <ThemeProvider scheme={scheme()}>
    <scroll class="selection-scroll">
      <column class="selection-page">
        <row class="selection-header">
          <text class="selection-heading">Selection controls</text>
          <Button testId="selection-theme" size="sm" variant="outline" onClick={() => setScheme(value => value === "light" ? "dark" : "light")}>Theme</Button>
        </row>
        <row class="selection-columns">
          <column class="selection-column">
            <column class="selection-section">
              <row class="selection-row">
                <text class="selection-title">Quality</text>
                <Button testId="before-radio" size="sm" variant="outline">Focus group</Button>
              </row>
              <RadioGroup testId="quality" value={quality()} onValueChange={setQuality} label="Quality">
                <Option id="quality-low" value="low" label="Economy" disabled />
                <Option id="quality-balanced" value="balanced" label="Balanced" />
                <Option id="quality-high" value="high" label="High" />
              </RadioGroup>
              <text testId="quality-value">Quality: {quality()}</text>
            </column>
            <column class="selection-section">
              <text class="selection-title">Alignment</text>
              <ToggleGroup testId="alignment" defaultValue={["left"]} onValueChange={setAlignment} label="Alignment" variant="outline" size="sm">
                <ToggleGroupItem testId="align-left" value="left" ariaLabel="Align left">Left</ToggleGroupItem>
                <ToggleGroupItem testId="align-center" value="center" ariaLabel="Align center">Center</ToggleGroupItem>
                <ToggleGroupItem testId="align-right" value="right" ariaLabel="Align right">Right</ToggleGroupItem>
              </ToggleGroup>
              <text testId="alignment-value">Alignment: {alignment().join(", ") || "none"}</text>
            </column>
            <column class="selection-section">
              <row class="selection-row">
                <text class="selection-title">Formatting</text>
                <Button testId="rotate-format" size="sm" variant="outline" onClick={() => setOrder(items => [items[items.length - 1], ...items.slice(0, -1)])}>Rotate</Button>
              </row>
              <ToggleGroup testId="formatting" selection="multiple" onValueChange={setFormatting} label="Formatting" variant="outline" size="sm">
                <For each={order().filter(value => italicShown() || value !== "italic")}>
                  {value => <ToggleGroupItem testId={`format-${value}`} value={value}>{names[value]}</ToggleGroupItem>}
                </For>
              </ToggleGroup>
              <text testId="formatting-value">Formatting: {formatting().join(", ") || "none"}</text>
              <Button testId="italic-toggle" size="sm" variant="outline" onClick={() => setItalicShown(value => !value)}>{italicShown() ? "Remove italic" : "Restore italic"}</Button>
            </column>
          </column>
          <column class="selection-column">
            <column class="selection-section">
              <text class="selection-title">Volume</text>
              <Show when={shown()}>
                <Slider testId="volume" label="Volume" value={volume()} onValueChange={setVolume} min={0} max={100} step={5} disabled={disabled()}
                  onPointerDown={event => setPointer(`${event.pointerKind}:${event.pointerId}:${event.primary}`)}
                  onPointerMove={() => setMoves(value => value + 1)} />
              </Show>
              <text testId="volume-value">Volume: {volume()}</text>
              <text testId="pointer-value">Pointer: {pointer()}</text>
              <text testId="pointer-moves">Moves: {moves()}</text>
              <row class="selection-row">
                <Button testId="disable-slider" size="sm" variant="outline" onClick={() => setDisabled(value => !value)}>{disabled() ? "Enable" : "Disable"}</Button>
                <Button testId="show-slider" size="sm" variant="outline" onClick={() => setShown(value => !value)}>{shown() ? "Hide" : "Show"}</Button>
              </row>
              <text class="selection-hint">Drag beyond the track; use the arrow and page keys.</text>
            </column>
            <column class="selection-section">
              <text class="selection-title">Scroll focus</text>
              <scroll class="selection-overflow" testId="overflow">
                <RadioGroup label="Overflow navigation" defaultValue="first">
                  <Option id="overflow-first" value="first" label="First" />
                  <Option id="overflow-second" value="second" label="Second" />
                  <Option id="overflow-third" value="third" label="Third" />
                  <Option id="overflow-fourth" value="fourth" label="Fourth" />
                  <Option id="overflow-last" value="last" label="Last" />
                </RadioGroup>
              </scroll>
              <text class="selection-hint">Home and End reveal the focused item.</text>
            </column>
          </column>
        </row>
      </column>
    </scroll>
  </ThemeProvider>;
}
