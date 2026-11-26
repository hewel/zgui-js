import { createSignal } from "@hewel/zgui";
import {
  Alert,
  AlertDescription,
  AlertTitle,
  AspectRatio,
  Button,
  Icon,
  Progress,
  Separator,
  Skeleton,
  Spinner,
} from "@hewel/zgui/components";
import {
  ALERT_CIRCLE,
  ALERT_TRIANGLE,
  CHECK,
  CHECK_CIRCLE,
  CROSS,
  ELLIPSIS,
  INFO,
  MINUS,
  PANEL_LEFT,
  PLUS,
  SEARCH,
} from "@hewel/zgui/icons";
import { DemoRow, Panel } from "../shell.js";

export function FeedbackSection(props: { onAction: (label: string) => void }) {
  const [progress, setProgress] = createSignal(64);

  return (
    <row class="panel-grid" testId="section-feedback">
      <Panel
        title="Alert & Progress"
        note="Inline status callouts plus determinate and indeterminate progress bars"
      >
        <column class="demo-stack">
          <Alert>
            <AlertTitle>Native host active</AlertTitle>
            <AlertDescription>
              Stylesheets, keyboard roving, and overlays run directly inside QuickJS.
            </AlertDescription>
          </Alert>
          <Alert variant="destructive">
            <AlertTitle>Budget guard enabled</AlertTitle>
            <AlertDescription>
              Single-turn JS work stays capped at 25 ms with clean session teardown.
            </AlertDescription>
          </Alert>
        </column>
        <DemoRow label="Determinate & indeterminate progress">
          <column class="demo-stack">
            <Progress
              testId="atoms-progress"
              value={progress()}
              max={100}
              label="Upload progress"
            />
            <row class="demo-items">
              <Button
                testId="progress-minus"
                size="sm"
                variant="outline"
                onClick={() => {
                  const next = Math.max(0, progress() - 15);
                  setProgress(next);
                  props.onAction(`Progress ${next}%`);
                }}
              >
                -15%
              </Button>
              <Button
                testId="progress-plus"
                size="sm"
                variant="outline"
                onClick={() => {
                  const next = Math.min(100, progress() + 15);
                  setProgress(next);
                  props.onAction(`Progress ${next}%`);
                }}
              >
                +15%
              </Button>
              <text testId="atoms-progress-text">{`${progress()}%`}</text>
            </row>
            <Progress value={null} label="Background sync" />
          </column>
        </DemoRow>
      </Panel>

      <Panel
        title="Icon, Spinner, Skeleton & AspectRatio"
        note="Vector icon geometry, loading indicators, placeholders, and proportion box"
      >
        <DemoRow label="Icon gallery">
          <Icon icon={CHECK} label="Check" />
          <Icon icon={CROSS} label="Cross" />
          <Icon icon={PLUS} label="Plus" />
          <Icon icon={MINUS} label="Minus" />
          <Icon icon={SEARCH} label="Search" />
          <Icon icon={INFO} label="Info" />
          <Icon icon={CHECK_CIRCLE} label="OK" />
          <Icon icon={ALERT_CIRCLE} label="Alert" />
          <Icon icon={ALERT_TRIANGLE} label="Warn" />
          <Icon icon={ELLIPSIS} label="More" />
          <Icon icon={PANEL_LEFT} label="Sidebar" />
        </DemoRow>
        <DemoRow label="Icon sizes & spinner">
          <Icon icon={CHECK_CIRCLE} size="sm" label="Small icon" />
          <Icon icon={CHECK_CIRCLE} size="md" label="Medium icon" />
          <Icon icon={CHECK_CIRCLE} size="lg" label="Large icon" />
          <Icon icon={CHECK_CIRCLE} size="xl" label="Extra large icon" />
          <Separator orientation="vertical" />
          <Spinner label="Loading" />
        </DemoRow>
        <DemoRow label="Skeleton lines">
          <column class="demo-stack">
            <Skeleton class="skeleton-line skeleton-w-78" />
            <Skeleton class="skeleton-line skeleton-w-100" />
            <Skeleton class="skeleton-line skeleton-w-46" />
          </column>
        </DemoRow>
        <DemoRow label="AspectRatio (16:9)">
          <box class="ratio-box">
            <AspectRatio ratio={16 / 9}>
              <box class="ratio-fill">
                <text>16 : 9 container</text>
              </box>
            </AspectRatio>
          </box>
        </DemoRow>
      </Panel>
    </row>
  );
}
