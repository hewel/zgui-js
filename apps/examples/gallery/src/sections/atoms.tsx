import {
  Badge, Button, ButtonGroup, ButtonGroupSeparator, ButtonGroupText,
  Icon, Kbd, KbdGroup, Separator, Spinner,
} from "@hewel/zgui/components";
import { PLUS } from "@hewel/zgui/icons";
import { DemoRow, Panel } from "../shell.js";

export function AtomsSection(props: { onAction: (label: string) => void }) {
  return (
    <row class="panel-grid" testId="section-atoms">
      <Panel
        title="Button & ButtonGroup"
        note="Six variants, sizes, icon button, loading state, and joined groups"
      >
        <DemoRow label="Variants">
          <Button testId="btn-default" onClick={() => props.onAction("Button: Default")}>Default</Button>
          <Button variant="secondary" onClick={() => props.onAction("Button: Secondary")}>Secondary</Button>
          <Button variant="destructive" onClick={() => props.onAction("Button: Destructive")}>Destructive</Button>
          <Button variant="outline" onClick={() => props.onAction("Button: Outline")}>Outline</Button>
          <Button variant="ghost" onClick={() => props.onAction("Button: Ghost")}>Ghost</Button>
          <Button variant="link" onClick={() => props.onAction("Button: Link")}>Link</Button>
        </DemoRow>
        <DemoRow label="Sizes, icon & loading">
          <Button size="xs" variant="outline">XS</Button>
          <Button size="sm" variant="outline">Small</Button>
          <Button size="lg" variant="outline">Large</Button>
          <Button size="icon" variant="outline" ariaLabel="Add"><Icon icon={PLUS} /></Button>
          <Button size="sm" onClick={() => props.onAction("Create item")}>
            <Icon icon={PLUS} size="sm" />
            <text>Create</text>
          </Button>
          <Button disabled>
            <Spinner />
            <text>Syncing</text>
          </Button>
        </DemoRow>
        <DemoRow label="ButtonGroup">
          <ButtonGroup>
            <Button variant="outline" size="sm" onClick={() => props.onAction("Prev")}>Prev</Button>
            <ButtonGroupText>2 / 5</ButtonGroupText>
            <Button variant="outline" size="sm" onClick={() => props.onAction("Next")}>Next</Button>
          </ButtonGroup>
          <ButtonGroup>
            <Button variant="secondary" size="sm" onClick={() => props.onAction("Copy")}>Copy</Button>
            <ButtonGroupSeparator />
            <Button variant="secondary" size="sm" onClick={() => props.onAction("Paste")}>Paste</Button>
          </ButtonGroup>
        </DemoRow>
      </Panel>

      <Panel
        title="Badge, Kbd & Separator"
        note="Status tones, keystroke legends, and horizontal/vertical dividers"
      >
        <DemoRow label="Badge variants">
          <Badge>Default</Badge>
          <Badge variant="secondary">Secondary</Badge>
          <Badge variant="destructive">Destructive</Badge>
          <Badge variant="success">Success</Badge>
          <Badge variant="warning">Warning</Badge>
          <Badge variant="outline">Outline</Badge>
          <Badge variant="ghost">Ghost</Badge>
          <Badge variant="link">Link</Badge>
        </DemoRow>
        <DemoRow label="Keyboard shortcuts">
          <KbdGroup>
            <Kbd>Ctrl</Kbd>
            <Kbd>K</Kbd>
          </KbdGroup>
          <Kbd>Esc</Kbd>
        </DemoRow>
        <DemoRow label="Separators">
          <column class="demo-stack">
            <text>Upper group</text>
            <Separator />
            <row style={{ height: "20px", "align-items": "center", gap: "10px" }}>
              <text>Overview</text>
              <Separator orientation="vertical" />
              <text>Tokens</text>
            </row>
          </column>
        </DemoRow>
      </Panel>
    </row>
  );
}
