import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Alert, AlertDescription, AlertTitle, Button, Checkbox, Input, Label,
  Progress, Skeleton, Spinner, Switch, Textarea, ThemeProvider, Toggle,
} from "@hewel/zgui/components";
import type { AlertVariant, CheckedState } from "@hewel/zgui/components";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Form />, root);
}

function Form() {
  const [name, setName] = createSignal("");
  const [checkbox, setCheckbox] = createSignal<CheckedState>("mixed");
  const [checkboxEvents, setCheckboxEvents] = createSignal(0);
  const [switched, setSwitched] = createSignal(false);
  const [switchEvents, setSwitchEvents] = createSignal(0);
  const [pressed, setPressed] = createSignal(false);
  const [alertIcon, setAlertIcon] = createSignal(true);
  const [alertVariant, setAlertVariant] = createSignal<AlertVariant>("default");
  const [alertBodyPressed, setAlertBodyPressed] = createSignal(false);
  let nameLabel: NativeNode | undefined;
  let nameInput: NativeNode | undefined;

  return (
    <ThemeProvider scheme="light">
      <scroll class="form-scroll">
        <row class="form">
          <column class="form-column">
            <Label testId="name-label" ref={node => { nameLabel = node; }}
              control={() => nameInput}>Name</Label>
            <Input testId="name-input" ref={node => { nameInput = node; }}
              ariaLabelledBy={nameLabel}
              value={name()} onValueChange={setName} />
            <text testId="name-value">Name: {name()}</text>
            <row class="form-row">
              <Checkbox testId="checkbox" ariaLabel="Checkbox" defaultChecked="mixed"
                onCheckedChange={value => {
                  setCheckbox(value);
                  setCheckboxEvents(count => count + 1);
                }} />
              <text testId="checkbox-state">Checkbox: {String(checkbox())}</text>
              <text testId="checkbox-events">{checkboxEvents()}</text>
            </row>
            <row class="form-row">
              <Switch testId="switch" ariaLabel="Switch" checked={switched()}
                onCheckedChange={value => {
                  setSwitched(value);
                  setSwitchEvents(count => count + 1);
                }} />
              <text testId="switch-state">Switch: {String(switched())}</text>
              <text testId="switch-events">{switchEvents()}</text>
            </row>
            <Toggle pressed={pressed()} label="Toggle"
              onPressedChange={setPressed}>Toggle me</Toggle>
          </column>
          <column class="form-column">
            <Input testId="password" label="Password" secure defaultValue="secret" />
            <Input testId="password-reversed" label="Password again" defaultValue="secret" secure />
            <Input testId="placeholder" label="Email" placeholder="you@example.com" />
            <Textarea testId="readonly" defaultValue="Read only" readOnly label="Notes" />
            <Progress testId="progress" value={40} label="Progress" />
            <Alert testId="alert" variant={alertVariant()} icon={alertIcon()}>
              <AlertTitle testId="alert-title">Ready</AlertTitle>
              <AlertDescription>
                All controls are live.
                <Toggle testId="alert-body-toggle" label="Inside"
                  onPressedChange={setAlertBodyPressed}>Inside</Toggle>
              </AlertDescription>
            </Alert>
            <text testId="alert-body-state">{String(alertBodyPressed())}</text>
            <row class="form-row">
              <Button testId="alert-icon-toggle" size="sm" variant="outline"
                onClick={() => setAlertIcon(value => !value)}>Icon</Button>
              <Button testId="alert-variant-toggle" size="sm" variant="outline"
                onClick={() => setAlertVariant(value => value === "default" ? "destructive" : "default")}>Variant</Button>
            </row>
            <row class="form-row">
              <Spinner testId="spinner" />
              <Skeleton testId="skeleton" style={{ width: "80px", height: "16px" }} />
            </row>
          </column>
        </row>
      </scroll>
    </ThemeProvider>
  );
}
