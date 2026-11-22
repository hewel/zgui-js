import { createSignal } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Icon,
  Input,
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
  InputGroupTextarea,
  Label,
  Textarea,
} from "@hewel/zgui/components";
import { SEARCH } from "@hewel/zgui/icons";
import { DemoRow, Panel } from "../shell.js";

export function FieldsSection(props: { onAction: (label: string) => void }) {
  const [name, setName] = createSignal("Ada Lovelace");
  const [notes, setNotes] = createSignal("Supports native selection, IME, and undo/redo.");
  const [query, setQuery] = createSignal("");
  const [comment, setComment] = createSignal("");
  const [invalidGroup, setInvalidGroup] = createSignal(false);

  let nameInput: NativeNode | undefined;

  return (
    <row class="panel-grid" testId="section-fields">
      <Panel
        title="Label, Input & Textarea"
        note="Native editing fields: controlled, placeholder, password, invalid, and disabled"
      >
        <column class="demo-stack">
          <Label control={() => nameInput}>Display name</Label>
          <Input
            testId="inputs-name"
            ref={el => {
              nameInput = el;
            }}
            label="Display name"
            value={name()}
            onValueChange={value => {
              setName(value);
              props.onAction(`Name edited (${value.length})`);
            }}
          />
          <text class="muted-caption">{`Current: ${name()}`}</text>
        </column>
        <DemoRow label="States">
          <Input placeholder="you@domain.dev" label="Email placeholder" />
          <Input defaultValue="hunter2" secure label="Password input" />
          <Input defaultValue="bad-token" invalid label="Invalid input" />
          <Input defaultValue="Read-only host" disabled label="Disabled input" />
        </DemoRow>
        <column class="demo-stack">
          <text class="demo-row-label">Multiline Textarea</text>
          <Textarea
            testId="inputs-notes"
            label="Release notes"
            placeholder="Write release notes..."
            value={notes()}
            onValueChange={setNotes}
          />
        </column>
      </Panel>

      <Panel
        title="InputGroup"
        note="Inline icons, static addons, buttons, invalid frame, and multiline composer"
      >
        <column class="demo-stack">
          <text class="demo-row-label">Search bar with leading icon & trailing action</text>
          <InputGroup>
            <InputGroupAddon>
              <Icon icon={SEARCH} size="sm" />
            </InputGroupAddon>
            <InputGroupInput
              value={query()}
              onValueChange={setQuery}
              placeholder="Filter components..."
              label="Filter components"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton onClick={() => props.onAction(`Search: ${query() || "(empty)"}`)}>
                Go
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        </column>
        <column class="demo-stack">
          <text class="demo-row-label">Static suffix & invalid state toggle</text>
          <row class="demo-items">
            <box style={{ "flex-grow": "1", "min-width": "200px" }}>
              <InputGroup invalid={invalidGroup()}>
                <InputGroupInput defaultValue="zgui-js" label="Repository name" />
                <InputGroupAddon align="inline-end">
                  <InputGroupText>.git</InputGroupText>
                </InputGroupAddon>
              </InputGroup>
            </box>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                const next = !invalidGroup();
                setInvalidGroup(next);
                props.onAction(next ? "InputGroup invalid" : "InputGroup valid");
              }}
            >
              {invalidGroup() ? "Clear invalid" : "Mark invalid"}
            </Button>
          </row>
        </column>
        <column class="demo-stack">
          <text class="demo-row-label">Textarea with block-end bar</text>
          <InputGroup>
            <InputGroupTextarea
              value={comment()}
              onValueChange={setComment}
              placeholder="Write a review comment..."
              label="Review comment"
            />
            <InputGroupAddon align="block-end">
              <InputGroupText>
                <text>{`${comment().length} chars`}</text>
              </InputGroupText>
              <InputGroupButton
                variant="default"
                size="xs"
                onClick={() => props.onAction(`Comment posted (${comment().length})`)}
              >
                Post
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        </column>
      </Panel>
    </row>
  );
}
