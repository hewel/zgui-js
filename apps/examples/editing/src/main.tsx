import { Button, TextArea, TextField, createSignal, renderApp } from "@zgui/ui";
import type { NativeNode, NativeValueEvent } from "@zgui/ui";

function Editing() {
  const [fieldValue, setFieldValue] = createSignal("A中B");
  const [areaValue, setAreaValue] = createSignal("one\n二");
  const [defaultValue, setDefaultValue] = createSignal("seed");
  const [inputCount, setInputCount] = createSignal(0);
  const [changeCount, setChangeCount] = createSignal(0);
  const [lastValue, setLastValue] = createSignal("");
  const [lastSelection, setLastSelection] = createSignal("0:0");
  const [targetValid, setTargetValid] = createSignal(true);

  function observe(event: NativeValueEvent) {
    setLastValue(event.value);
    setLastSelection(`${event.selection.start}:${event.selection.end}`);
    setTargetValid(valid => valid && event.target === event.currentTarget);
  }
  function input(event: NativeValueEvent) {
    setInputCount(count => count + 1);
    observe(event);
  }
  function change(event: NativeValueEvent) {
    setChangeCount(count => count + 1);
    observe(event);
  }

  return <column class="editing" style={{ gap: "12px" }}>
    <text class="editing-heading">Native text editing</text>
    <column class="editing-group">
      <text>Controlled single-line field</text>
      <TextField class="editing-field" ariaLabel="Controlled single-line field"
        value={fieldValue()} onInput={event => { setFieldValue(event.value); input(event); }} onChange={change} />
    </column>
    <column class="editing-group">
      <text>Controlled multiline area</text>
      <TextArea class="editing-area" ariaLabel="Controlled multiline area"
        value={areaValue()} onInput={event => { setAreaValue(event.value); input(event); }} onChange={change} />
    </column>
    <column class="editing-group">
      <text>Uncontrolled field (initial value only)</text>
      <TextField class="editing-default" ariaLabel="Uncontrolled field" defaultValue={defaultValue()}
        onInput={input} onChange={change} />
    </column>
    <row class="editing-actions">
      <Button class="editing-reset" onClick={() => setFieldValue("Reset 中")}>Reset controlled field</Button>
      <Button class="editing-change-default" onClick={() => setDefaultValue("replacement")}>Change default source</Button>
    </row>
    <row class="editing-states">
      <column class="editing-group">
        <text>Read only (selectable)</text>
        <TextField class="editing-readonly" ariaLabel="Read-only field" value="Read only" readOnly
          onInput={input} onChange={change} />
      </column>
      <column class="editing-group">
        <text>Disabled</text>
        <TextField class="editing-disabled" ariaLabel="Disabled field" value="Disabled" disabled
          onInput={input} onChange={change} />
      </column>
    </row>
    <column class="editing-report">
      <text>Events across all five controls</text>
      <row><text>Input events: </text><text class="editing-input-count">{inputCount()}</text></row>
      <row><text>Committed changes: </text><text class="editing-change-count">{changeCount()}</text></row>
      <row><text>Last native value: </text><text class="editing-last-value">{lastValue()}</text></row>
      <row><text>Selection (UTF-8 bytes): </text><text class="editing-last-selection">{lastSelection()}</text></row>
      <row><text>Event target is control: </text><text class="editing-target-valid">{String(targetValid())}</text></row>
    </column>
  </column>;
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Editing />, root);
}
