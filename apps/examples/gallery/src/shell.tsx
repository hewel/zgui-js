import type { NativeChild, NativeNode } from "@hewel/zgui";
import {
  Badge,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  Label,
  MenuLabel,
  MenuRadioGroup,
  MenuRadioItem,
  MenuSeparator,
  Switch,
} from "@hewel/zgui/components";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";

export const PRESETS: readonly ThemePreset[] = ["base", "ocean", "ember", "midnight", "terminal"];
export const PRESET_LABELS: Record<ThemePreset, string> = {
  base: "Base",
  ocean: "Ocean",
  ember: "Ember",
  midnight: "Midnight",
  terminal: "Terminal",
};

export const SECTIONS = [
  "atoms",
  "feedback",
  "fields",
  "choices",
  "composites",
  "overlays",
  "menus",
  "workbench",
] as const;
export type SectionId = (typeof SECTIONS)[number];
export const SECTION_LABELS: Record<SectionId, string> = {
  atoms: "Atoms",
  feedback: "Feedback & Media",
  fields: "Fields",
  choices: "Choices",
  composites: "Composites",
  overlays: "Overlays",
  menus: "Menus",
  workbench: "Workbench",
};

export function Panel(props: {
  title: string;
  note: string;
  wide?: boolean;
  testId?: string;
  children?: NativeChild;
}) {
  return (
    <column class={props.wide ? "panel panel-wide" : "panel"} testId={props.testId}>
      <text class="panel-title">{props.title}</text>
      <text class="panel-note">{props.note}</text>
      <column class="panel-body">{props.children}</column>
    </column>
  );
}

export function DemoRow(props: { label: string; children?: NativeChild }) {
  return (
    <column class="demo-row">
      <text class="demo-row-label">{props.label}</text>
      <row class="demo-items">{props.children}</row>
    </column>
  );
}

export function Masthead(props: {
  scheme: () => ColorScheme;
  setScheme: (value: ColorScheme) => void;
  preset: () => ThemePreset;
  setPreset: (value: ThemePreset) => void;
  lastAction: () => string;
  onAction: (label: string) => void;
}) {
  let switchNode: NativeNode | undefined;

  return (
    <row class="masthead">
      <column class="masthead-title-group">
        <text class="masthead-title">zgui/ui Component Gallery</text>
        <text class="masthead-subtitle">
          All ported @hewel/zgui/components showcased on native QuickJS + zgui
        </text>
      </column>
      <row class="masthead-controls">
        <Badge testId="last-action" variant="secondary">
          <text>{`Last: ${props.lastAction()}`}</text>
        </Badge>
        <DropdownMenu>
          <DropdownMenuTrigger testId="preset-trigger" size="sm" variant="outline">
            <text>{`Preset: ${PRESET_LABELS[props.preset()]}`}</text>
          </DropdownMenuTrigger>
          <DropdownMenuContent testId="preset-menu" style="width: 180px">
            <MenuLabel>Theme preset</MenuLabel>
            <MenuSeparator />
            <MenuRadioGroup
              value={props.preset()}
              onValueChange={value => {
                const next = value as ThemePreset;
                props.setPreset(next);
                props.onAction(`Preset ${PRESET_LABELS[next]}`);
              }}
            >
              {PRESETS.map(name => (
                <MenuRadioItem testId={`preset-${name}`} value={name}>
                  {PRESET_LABELS[name]}
                </MenuRadioItem>
              ))}
            </MenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <row class="demo-pair">
          <Label control={() => switchNode}>Dark</Label>
          <Switch
            testId="scheme-switch"
            ref={node => {
              switchNode = node;
            }}
            checked={props.scheme() === "dark"}
            onCheckedChange={on => {
              const next: ColorScheme = on ? "dark" : "light";
              props.setScheme(next);
              props.onAction(`Scheme ${next}`);
            }}
            ariaLabel="Toggle dark color scheme"
          />
        </row>
        <Badge testId="scheme-badge" variant="outline">
          <text>{`${props.preset()} / ${props.scheme()}`}</text>
        </Badge>
      </row>
    </row>
  );
}
