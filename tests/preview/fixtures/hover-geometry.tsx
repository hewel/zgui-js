import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  ThemeProvider,
  Tooltip,
  TooltipArrow,
  TooltipContent,
  TooltipTrigger,
} from "@hewel/zgui/components";
import type { PopperSide } from "@hewel/zgui/components";
export const hostApiVersion = 1;
function Hint(props: { id: string; x: number; y: number; side: PopperSide }) {
  return (
    <Tooltip defaultOpen>
      <TooltipTrigger
        testId={props.id + "-anchor"}
        style={`position: fixed; left: ${props.x}px; top: ${props.y}px; width: 100px; height: 36px`}
      >
        <Button style="width: 100px; height: 36px">{props.id}</Button>
      </TooltipTrigger>
      <TooltipContent
        testId={props.id + "-surface"}
        arrow={false}
        offset={(10 * Math.SQRT2) / 2 + 3}
        placement={{ side: props.side, align: "center" }}
        style="width: 160px"
      >
        <text>{props.id} hint</text>
        <TooltipArrow testId={props.id + "-arrow"} />
      </TooltipContent>
    </Tooltip>
  );
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dark, setDark] = createSignal(false);
    return (
      <ThemeProvider scheme={dark() ? "dark" : "light"}>
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <Hint id="flipped" x={20} y={20} side="top" />
          <Hint id="top" x={300} y={220} side="top" />
          <Hint id="left" x={260} y={380} side="left" />
          <Hint id="right" x={500} y={380} side="right" />
          <Button
            testId="theme"
            style="position: fixed; left: 20px; top: 520px"
            onClick={() => setDark(value => !value)}
          >
            Toggle theme
          </Button>
        </box>
      </ThemeProvider>
    );
  }, root);
}
