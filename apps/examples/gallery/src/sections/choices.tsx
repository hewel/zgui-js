import { createSignal } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Checkbox,
  Label,
  RadioGroup,
  RadioGroupItem,
  Slider,
  Switch,
  Toggle,
  ToggleGroup,
  ToggleGroupItem,
} from "@hewel/zgui/components";
import type { CheckedState } from "@hewel/zgui/components";
import { DemoRow, Panel } from "../shell.js";

function RadioOption(props: { value: string; label: string; disabled?: boolean }) {
  let node: NativeNode | undefined;
  return (
    <row class="demo-pair">
      <RadioGroupItem
        ref={el => {
          node = el;
        }}
        value={props.value}
        label={props.label}
        disabled={props.disabled}
      />
      <Label control={() => node} disabled={props.disabled}>
        {props.label}
      </Label>
    </row>
  );
}

export function ChoicesSection(props: { onAction: (label: string) => void }) {
  const [terms, setTerms] = createSignal<CheckedState>(true);
  const [mixed, setMixed] = createSignal<CheckedState>("mixed");
  const [emails, setEmails] = createSignal(true);
  const [channel, setChannel] = createSignal("stable");
  const [bold, setBold] = createSignal(true);
  const [align, setAlign] = createSignal<readonly string[]>(["left"]);
  const [formats, setFormats] = createSignal<readonly string[]>(["bold"]);
  const [volume, setVolume] = createSignal(45);

  let termsBox: NativeNode | undefined;
  let switchNode: NativeNode | undefined;

  return (
    <row class="panel-grid" testId="section-choices">
      <Panel
        title="Checkbox, Switch & RadioGroup"
        note="Tri-state checkboxes, compact/standard switches, and roving radio groups"
      >
        <DemoRow label="Checkbox states">
          <row class="demo-pair">
            <Checkbox
              testId="inputs-terms"
              ref={el => {
                termsBox = el;
              }}
              checked={terms()}
              onCheckedChange={value => {
                setTerms(value);
                props.onAction(`Terms: ${String(value)}`);
              }}
              ariaLabel="Accept terms"
            />
            <Label control={() => termsBox}>Accept terms</Label>
          </row>
          <Checkbox checked={mixed()} onCheckedChange={setMixed} ariaLabel="Mixed checkbox" />
          <Button
            size="xs"
            variant="outline"
            onClick={() =>
              setMixed(cur => (cur === false ? "mixed" : cur === "mixed" ? true : false))
            }
          >
            <text>{`Mixed: ${String(mixed())}`}</text>
          </Button>
          <Checkbox defaultChecked={false} disabled ariaLabel="Disabled checkbox" />
        </DemoRow>
        <DemoRow label="Switch">
          <row class="demo-pair">
            <Switch
              ref={el => {
                switchNode = el;
              }}
              checked={emails()}
              onCheckedChange={value => {
                setEmails(value);
                props.onAction(`Digest ${value ? "on" : "off"}`);
              }}
              ariaLabel="Weekly digest"
            />
            <Label control={() => switchNode}>Weekly digest</Label>
          </row>
          <Switch size="sm" defaultChecked ariaLabel="Compact switch" />
          <Switch checked={emails()} disabled ariaLabel="Disabled switch" />
        </DemoRow>
        <DemoRow label="RadioGroup">
          <column class="demo-stack">
            <RadioGroup
              value={channel()}
              onValueChange={value => {
                setChannel(value);
                props.onAction(`Channel: ${value}`);
              }}
              label="Release channel"
            >
              <row class="demo-items">
                <RadioOption value="stable" label="Stable" />
                <RadioOption value="beta" label="Beta" />
                <RadioOption value="nightly" label="Nightly (locked)" disabled />
              </row>
            </RadioGroup>
            <text class="muted-caption">{`Selected channel: ${channel()}`}</text>
          </column>
        </DemoRow>
      </Panel>

      <Panel
        title="Toggle, ToggleGroup & Slider"
        note="Stateful press toggles, single/multi selection groups, and step slider"
      >
        <DemoRow label="Standalone toggles">
          <Toggle pressed={bold()} onPressedChange={setBold} ariaLabel="Bold">
            B
          </Toggle>
          <Toggle variant="outline" defaultPressed ariaLabel="Italic">
            I
          </Toggle>
          <Toggle size="sm" variant="outline" ariaLabel="Small toggle">
            Sm
          </Toggle>
          <Toggle size="lg" variant="outline" ariaLabel="Large toggle">
            Lg
          </Toggle>
          <Toggle disabled ariaLabel="Disabled toggle">
            Off
          </Toggle>
        </DemoRow>
        <DemoRow label="ToggleGroup (single vs multiple)">
          <ToggleGroup
            selection="single"
            variant="outline"
            size="sm"
            value={align()}
            onValueChange={value => {
              setAlign(value);
              props.onAction(`Align: ${value.join(",") || "none"}`);
            }}
            label="Alignment"
          >
            <ToggleGroupItem value="left">Left</ToggleGroupItem>
            <ToggleGroupItem value="center">Center</ToggleGroupItem>
            <ToggleGroupItem value="right">Right</ToggleGroupItem>
          </ToggleGroup>
          <ToggleGroup
            selection="multiple"
            variant="outline"
            size="sm"
            value={formats()}
            onValueChange={setFormats}
            label="Formatting"
          >
            <ToggleGroupItem value="bold">B</ToggleGroupItem>
            <ToggleGroupItem value="italic">I</ToggleGroupItem>
            <ToggleGroupItem value="code">Code</ToggleGroupItem>
          </ToggleGroup>
        </DemoRow>
        <DemoRow label="Slider (drag or Arrow keys)">
          <column class="demo-stack">
            <Slider
              testId="inputs-slider"
              value={volume()}
              onValueChange={value => {
                setVolume(value);
                props.onAction(`Gain ${value}%`);
              }}
              min={0}
              max={100}
              step={5}
              label="Output gain"
            />
            <text testId="inputs-slider-text" class="muted-caption">{`Gain: ${volume()}%`}</text>
            <Slider value={volume()} min={0} max={100} disabled label="Locked slider" />
          </column>
        </DemoRow>
      </Panel>
    </row>
  );
}
