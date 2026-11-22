import { after, before, test } from "node:test";
import { buildFixtureApp, createTempWorkspace, runScenario } from "./harness.mjs";

const fixtureSource = `
import {
  Show,
  createSignal,
  createSpring,
  focus,
  renderApp,
  startInternalDrag,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { createDelayed, createRetainedPresence } from "@hewel/zgui/primitives";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const [payload, setPayload] = createSignal<{ readonly label: string } | null>({ label: "Panel" });
    const [lowerClicks, setLowerClicks] = createSignal(0);
    const [panelClicks, setPanelClicks] = createSignal(0);
    const [panelBlurs, setPanelBlurs] = createSignal(0);
    const [transitionEnds, setTransitionEnds] = createSignal(0);
    const [dragInert, setDragInert] = createSignal(false);
    const [dragEndReason, setDragEndReason] = createSignal("none");
    const [dropCount, setDropCount] = createSignal(0);

    const armDragInert = createDelayed();

    const progress = createSpring(() => (payload() ? 1 : 0), {
      initial: 1,
      stiffness: 180,
      damping: 24,
      precision: 0.01,
    });
    const presence = createRetainedPresence(payload, { settled: progress.settled });

    let panelBtn: NativeNode | undefined;
    let dragSourceNode: NativeNode | undefined;

    return (
      <box style="position: relative; width: 380px; height: 260px; padding: 8px;">
        <row style="gap: 6px; margin-bottom: 8px;">
          <control
            testId="btn-focus-panel"
            style="width: 72px; height: 28px;"
            onClick={() => {
              if (panelBtn) focus(panelBtn);
            }}
          >
            <text>Focus</text>
          </control>
          <control
            testId="btn-close"
            style="width: 72px; height: 28px;"
            onClick={() => setPayload(null)}
          >
            <text>Close</text>
          </control>
          <control
            testId="btn-open"
            style="width: 72px; height: 28px;"
            onClick={() => setPayload({ label: "Panel" })}
          >
            <text>Open</text>
          </control>
          <control
            testId="btn-inert-drag"
            style="width: 90px; height: 28px;"
            onClick={() => setDragInert(true)}
          >
            <text>InertDrag</text>
          </control>
        </row>

        <box style="position: relative; width: 220px; height: 70px;">
          <control
            testId="lower-btn"
            style="position: absolute; left: 0px; top: 0px; width: 200px; height: 60px; background-color: #2563eb;"
            onClick={() => setLowerClicks(c => c + 1)}
          >
            <text>Lower</text>
          </control>
          <Show when={presence.mounted()}>
            <box
              testId="exiting-panel"
              inert={!presence.present()}
              style={{
                position: "absolute",
                left: "0px",
                top: "0px",
                width: "200px",
                height: "60px",
                backgroundColor: "#dc2626",
                opacity: String(Math.max(0.05, Math.min(1, progress()))),
                transition: "background-color 80ms linear",
              }}
              onTransitionEnd={() => setTransitionEnds(n => n + 1)}
            >
              <control
                testId="panel-btn"
                ref={el => {
                  panelBtn = el;
                }}
                style="width: 180px; height: 48px;"
                onClick={() => setPanelClicks(c => c + 1)}
                onBlur={() => setPanelBlurs(n => n + 1)}
              >
                <text>{presence.value()?.label ?? ""}</text>
              </control>
            </box>
          </Show>
        </box>

        <row style="margin-top: 12px; gap: 12px;">
          <box testId="drag-host" inert={dragInert()}>
            <control
              testId="drag-source"
              ref={el => {
                dragSourceNode = el;
              }}
              style="width: 90px; height: 36px; background-color: #059669;"
              onPointerDown={() => {
                if (!dragSourceNode) return;
                startInternalDrag(dragSourceNode, {
                  types: ["text/plain"],
                  payload: "item-1",
                  onEnd: res => {
                    setDragEndReason(\`\${res.dropped ? "drop" : "cancel"}:\${res.reason}\`);
                  },
                });
                armDragInert.after(40, () => setDragInert(true));
              }}
            >
              <text>DragMe</text>
            </control>
          </box>
          <box
            testId="drop-target"
            style="width: 110px; height: 36px; background-color: #7c3aed;"
            onDragOver={ev => {
              ev.accept("move");
            }}
            onDrop={() => {
              setDropCount(n => n + 1);
            }}
          >
            <text>DropHere</text>
          </box>
        </row>

        <row style="margin-top: 8px; gap: 8px;">
          <text testId="readout-lower">{String(lowerClicks())}</text>
          <text testId="readout-panel">{String(panelClicks())}</text>
          <text testId="readout-blur">{String(panelBlurs())}</text>
          <text testId="readout-settled">{String(progress.settled())}</text>
          <text testId="readout-drag">{dragEndReason()}</text>
          <text testId="readout-drop">{String(dropCount())}</text>
          <text testId="readout-trans">{String(transitionEnds())}</text>
        </row>
      </box>
    );
  }, root);
}
`;

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-inert-");
  bundle = await buildFixtureApp(temp.dir, "inert-retained-exit", fixtureSource, {
    rawSource: true,
  });
});
after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `inert-step-${++serial}`;
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const clickAt = (x, y) => ({
  id: id(),
  op: "pointer.click",
  surface: "main",
  position: { x, y },
  button: "primary",
});
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(name, steps) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `inert-${name}`,
    width: 400,
    height: 280,
    scale: 1,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.keyboard",
      "tree.inspect",
    ],
    steps,
    assertEmptyResources: true,
  });
}

test("inert={!presence.present()} evacuates focus, passes clicks to lower sibling, and completes spring exit", async () => {
  await run("retained-exit", [
    // Initially open: panel covers lower-btn at (80, 96).
    check("exiting-panel", "exists", true),
    click("btn-focus-panel"),
    check("panel-btn", "focused", true),
    clickAt(80, 96),
    check("readout-panel", "text", "1"),
    check("readout-lower", "text", "0"),

    // Start retained exit -> inert becomes true immediately while panel stays mounted for spring!
    click("btn-close"),
    advance(16),
    check("exiting-panel", "exists", true),
    check("panel-btn", "focused", false),
    check("readout-blur", "text", "1"),

    // Mid-exit click at the EXACT overlapping coordinates (80, 96) passes through inert panel to lower-btn!
    clickAt(80, 96),
    check("readout-lower", "text", "1"),
    check("readout-panel", "text", "1"),

    // Reopen mid-exit -> inert clears immediately and panel intercepts clicks again!
    click("btn-open"),
    advance(16),
    check("exiting-panel", "exists", true),
    clickAt(80, 96),
    check("readout-panel", "text", "2"),
    check("readout-lower", "text", "1"),

    // Close again and advance virtual clock until spring settles and panel unmounts cleanly.
    click("btn-close"),
    advance(800),
    check("readout-settled", "text", "true"),
    check("exiting-panel", "exists", false),
  ]);
});

test("internal drag auto-cancels with reason inert when its still-mounted source becomes inert", async () => {
  await run("drag-source-inert", [
    // Press drag-source to start internal drag and arm the 40ms inert timer.
    {
      id: id(),
      op: "pointer.down",
      target: { surface: "main", testId: "drag-source" },
      button: "primary",
    },
    // Move over drop-target (it accepts).
    {
      id: id(),
      op: "pointer.move",
      target: { surface: "main", testId: "drop-target" },
    },
    // Advance clock past 40ms -> drag-host becomes inert while still mounted, cancelling the drag!
    advance(60),
    check("drag-source", "exists", true),
    check("readout-drag", "text", "cancel:inert"),
    // Release pointer over drop-target -> must NOT fire onDrop!
    {
      id: id(),
      op: "pointer.up",
      target: { surface: "main", testId: "drop-target" },
      button: "primary",
    },
    check("readout-drop", "text", "0"),
  ]);
});
