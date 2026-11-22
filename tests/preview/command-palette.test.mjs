import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
} from "./harness.mjs";
import { scoreCommandMatch } from "../../packages/ui/src/components/command-palette.ts";

const FIXTURE_SOURCE = `
import { createEffect, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  CommandPalette,
  CommandPaletteProvider,
  ThemeProvider,
  useCommandPalette,
} from "@hewel/zgui/components";
import type { CommandItem } from "@hewel/zgui/components";

function buildLargeCommandSet(record: (id: string) => void): readonly CommandItem[] {
  const base: CommandItem[] = [
    {
      id: "cmd-00",
      label: "Alpha Workspace Overview",
      group: "Navigation",
      shortcut: ["Ctrl", "P"],
      keywords: ["jump", "overview"],
      onSelect: () => record("cmd-00"),
    },
    {
      id: "cmd-01",
      label: "Disabled Maintenance Action",
      group: "Navigation",
      disabled: true,
      onSelect: () => record("cmd-01"),
    },
    {
      id: "cmd-02",
      label: "Open Workspace Settings",
      group: "Navigation",
      description: "Configure editor preferences",
      shortcut: "Ctrl+,",
      keywords: ["config", "prefs"],
      onSelect: () => record("cmd-02"),
    },
    {
      id: "cmd-03",
      label: "Toggle Split Editor",
      group: "Editor",
      shortcut: ["Ctrl", "\\\\"],
      onSelect: () => record("cmd-03"),
    },
  ];
  const groups = ["Editor", "Workspace", "Tools"];
  for (let i = 4; i < 40; i++) {
    const id = \`cmd-\${String(i).padStart(2, "0")}\`;
    base.push({
      id,
      label: \`Command Action \${String(i).padStart(2, "0")}\`,
      group: groups[i % groups.length],
      shortcut: i === 39 ? ["Alt", "End"] : undefined,
      disabled: i === 38,
      onSelect: () => record(id),
    });
  }
  return base;
}

function ScopeA(props: { record: (id: string) => void; setActive: (id: string | null) => void }) {
  const palette = useCommandPalette();
  createEffect(
    () => true,
    () => palette.registerCommand({
      id: "cmd-dyn-a",
      label: "Dynamic Scope A Probe",
      group: "Tools",
      shortcut: "F9",
      onSelect: () => props.record("cmd-dyn-a"),
    }),
  );
  return (
    <row style={{ gap: "8px" }}>
      <Button
        testId="cp-trigger-a"
        onClick={() => palette.setOpen(true)}
      >
        Open Palette A
      </Button>
      <CommandPalette
        testId="cp-dialog-a"
        inputTestId="cp-input-a"
        listTestId="cp-list-a"
        emptyTestId="cp-empty-a"
        placeholder="Search Scope A..."
        emptyText="No commands match"
        onActiveIdChange={props.setActive}
      />
    </row>
  );
}

function ScopeB(props: { record: (id: string) => void; setActive: (id: string | null) => void }) {
  const palette = useCommandPalette();
  createEffect(
    () => true,
    () => palette.registerCommands([
      {
        id: "scope-b-only",
        label: "Scope B Isolated Command",
        group: "Secondary",
        shortcut: ["Ctrl", "B"],
        onSelect: () => props.record("scope-b-only"),
      },
    ]),
  );
  return (
    <row style={{ gap: "8px" }}>
      <Button
        testId="cp-trigger-b"
        onClick={() => palette.setOpen(true)}
      >
        Open Palette B
      </Button>
      <CommandPalette
        testId="cp-dialog-b"
        inputTestId="cp-input-b"
        listTestId="cp-list-b"
        emptyTestId="cp-empty-b"
        placeholder="Search Scope B..."
        emptyText="Scope B empty"
        onActiveIdChange={props.setActive}
      />
    </row>
  );
}

function App() {
  const [log, setLog] = createSignal<string>("executions:0:none");
  const [active, setActive] = createSignal<string>("none");
  let count = 0;
  const record = (id: string) => {
    count += 1;
    setLog(\`executions:\${count}:\${id}\`);
  };
  const onActive = (id: string | null) => setActive(id ?? "none");
  const commandsA = buildLargeCommandSet(record);

  return (
    <ThemeProvider>
      <column style={{ width: "100%", height: "100%", padding: "20px", gap: "12px" }}>
        <label testId="cp-log">{log()}</label>
        <label testId="cp-active-id">{active()}</label>
        <CommandPaletteProvider commands={commandsA}>
          <ScopeA record={record} setActive={onActive} />
        </CommandPaletteProvider>
        <CommandPaletteProvider>
          <ScopeB record={record} setActive={onActive} />
        </CommandPaletteProvider>
      </column>
    </ThemeProvider>
  );
}

export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}
`;

let temp;
let bundle;

before(async () => {
  temp = await createTempWorkspace("preview-command-palette-");
  bundle = await buildFixtureApp(path.join(temp.dir, "app"), "command-palette", FIXTURE_SOURCE, {
    rawSource: true,
  });
});

after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const key = (name, modifiers) => ({
  id: id(),
  op: "key.press",
  surface: "main",
  key: name,
  ...(modifiers ? { modifiers } : {}),
});
const commitText = value => ({ id: id(), op: "text.commit", surface: "main", text: value });
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const capture = captureId => ({ id: id(), op: "capture", captureId });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const resize = (width, height) => ({
  id: id(),
  op: "surface.resize",
  surface: "main",
  width,
  height,
});

const CAPS = [
  "pixels",
  "virtual-time",
  "selectors.test-id",
  "input.pointer",
  "input.keyboard",
  "input.text",
  "tree.inspect",
  "surface.resize",
];

function run(name, steps) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `command-palette-${name}`,
    requiredCapabilities: CAPS,
    steps,
  });
}

function descendantsOf(tree, rootToken) {
  const byParent = new Map();
  for (const node of tree.nodes) {
    if (!node.parent) continue;
    let bucket = byParent.get(node.parent);
    if (!bucket) {
      bucket = [];
      byParent.set(node.parent, bucket);
    }
    bucket.push(node);
  }
  const out = [];
  const stack = [...(byParent.get(rootToken) ?? [])];
  while (stack.length > 0) {
    const cur = stack.shift();
    out.push(cur);
    const kids = byParent.get(cur.token);
    if (kids) stack.push(...kids);
  }
  return out;
}

function inspectList(tree, listTestId) {
  const list = getNode(tree, listTestId);
  const sub = descendantsOf(tree, list.token);
  const options = sub.filter(
    n => typeof n.role === "string" && n.role.toLowerCase().replaceAll("_", "") === "listboxoption",
  );
  const texts = sub
    .map(n => n.text ?? n.name)
    .filter(value => typeof value === "string" && value.length > 0);
  const scrollY = list.scrollMetrics?.offset?.y ?? list.scrollMetrics?.offsetY ?? 0;
  return { count: options.length, texts, joinedText: texts.join(" | "), scrollY };
}

test("scoreCommandMatch ranks exact > prefix > substring > fuzzy and rejects non-matches", () => {
  const exact = scoreCommandMatch({ id: "1", label: "save" }, "save");
  const prefix = scoreCommandMatch({ id: "2", label: "save all changes" }, "save");
  const sub = scoreCommandMatch({ id: "3", label: "auto save file" }, "save");
  const fuzzy = scoreCommandMatch({ id: "4", label: "show available views everywhere" }, "save");
  const miss = scoreCommandMatch({ id: "5", label: "open terminal" }, "save");
  const multi = scoreCommandMatch(
    { id: "6", label: "Open Workspace Settings", group: "Navigation" },
    "open nav",
  );

  assert.ok(exact !== null && prefix !== null && sub !== null && fuzzy !== null);
  assert.ok(
    exact > prefix && prefix > sub && sub > fuzzy,
    `unexpected ranking: ${exact}, ${prefix}, ${sub}, ${fuzzy}`,
  );
  assert.equal(miss, null);
  assert.ok(multi !== null && multi > 0);
});

test(
  "CommandPalette supports virtualized navigation, disabled skipping, focus retention, and Enter single-invocation",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("nav-and-invoke", [
      resize(900, 660),
      advance(60),
      click("cp-trigger-a"),
      advance(60),
      check("cp-dialog-a", "exists", true),
      check("cp-input-a", "focused", true),
      check("cp-active-id", "text", "cmd-00"),
      capture("open-initial"),

      // End jumps to the bottom of the 41-command virtualized list (scrollToIndex)
      key("End"),
      advance(40),
      check("cp-input-a", "focused", true),
      check("cp-active-id", "text", "cmd-dyn-a"),
      capture("after-end"),

      // Tools group ends with ..., cmd-32, cmd-35, cmd-38 (disabled), cmd-dyn-a.
      // ArrowUp skips disabled cmd-38 directly to cmd-35, then moves to cmd-32.
      key("ArrowUp"),
      advance(30),
      check("cp-active-id", "text", "cmd-35"),
      key("ArrowUp"),
      advance(30),
      check("cp-active-id", "text", "cmd-32"),
      check("cp-input-a", "focused", true),

      // Home returns to cmd-00; ArrowDown skips disabled cmd-01 directly to cmd-02
      key("Home"),
      advance(40),
      check("cp-active-id", "text", "cmd-00"),
      capture("after-home"),
      key("ArrowDown"),
      advance(30),
      check("cp-active-id", "text", "cmd-02"),
      check("cp-input-a", "focused", true),
      capture("at-cmd-02"),

      // Enter invokes cmd-02 once, closes dialog, and restores focus to trigger
      key("Enter"),
      advance(300),
      check("cp-log", "text", "executions:1:cmd-02"),
      check("cp-dialog-a", "exists", false),
      check("cp-trigger-a", "focused", true),
      capture("after-enter"),
    ]);

    const initialTree = await readTree(out, "open-initial");
    const initialList = inspectList(initialTree, "cp-list-a");
    assert.ok(
      initialList.joinedText.includes("Alpha Workspace Overview"),
      `expected cmd-00 rendered: ${initialList.joinedText}`,
    );
    assert.ok(
      initialList.joinedText.includes("Ctrl") && initialList.joinedText.includes("P"),
      `expected Kbd shortcut badges rendered: ${initialList.joinedText}`,
    );
    assert.ok(
      initialList.count > 0 && initialList.count < 25,
      `expected VirtualList windowing (<25 mounted options out of 41), got ${initialList.count}`,
    );

    const endTree = await readTree(out, "after-end");
    const endList = inspectList(endTree, "cp-list-a");
    assert.ok(
      endList.joinedText.includes("Dynamic Scope A Probe"),
      `expected bottom command mounted after End: ${endList.joinedText}`,
    );
    assert.ok(
      endList.scrollY > 200,
      `expected scrollToIndex to advance scrollY > 200 on End, got ${endList.scrollY}`,
    );

    const homeTree = await readTree(out, "after-home");
    const homeList = inspectList(homeTree, "cp-list-a");
    assert.equal(
      homeList.scrollY,
      0,
      `expected Home to scroll back to top, got ${homeList.scrollY}`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "CommandPalette preserves stable active commandId across filter updates, shows empty state, and restores focus on Escape",
  { timeout: 30000 },
  async () => {
    const { result } = await run("filter-escape", [
      resize(900, 660),
      advance(60),
      click("cp-trigger-a"),
      advance(60),
      check("cp-input-a", "focused", true),
      check("cp-active-id", "text", "cmd-00"),

      // Move activeId to cmd-02 ("Open Workspace Settings"), then filter by "Workspace".
      // Both cmd-00 ("Alpha Workspace Overview") and cmd-02 match, and activeId must stay pinned on cmd-02!
      key("ArrowDown"),
      advance(30),
      check("cp-active-id", "text", "cmd-02"),
      commitText("Workspace"),
      advance(40),
      check("cp-input-a", "focused", true),
      check("cp-active-id", "text", "cmd-02"),
      capture("filtered-preserve-active"),

      // Clear with Ctrl+A + Backspace and test fuzzy subsequence "ows" -> "Open Workspace Settings"
      key({ character: "a" }, ["Control"]),
      key("Backspace"),
      advance(20),
      commitText("ows"),
      advance(40),
      check("cp-input-a", "focused", true),
      check("cp-active-id", "text", "cmd-02"),
      capture("fuzzy-match"),

      // Query with zero matches renders empty state while keeping input focused
      commitText("zzzzz"),
      advance(40),
      check("cp-empty-a", "exists", true),
      check("cp-empty-a", "text", "No commands match"),
      check("cp-active-id", "text", "none"),
      check("cp-input-a", "focused", true),
      capture("empty-state"),

      // Escape dismisses and restores focus to trigger without invoking anything
      key("Escape"),
      advance(300),
      check("cp-dialog-a", "exists", false),
      check("cp-trigger-a", "focused", true),
      check("cp-log", "text", "executions:0:none"),
    ]);

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "CommandPaletteProvider isolates command registries across sibling providers",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("provider-isolation", [
      resize(900, 660),
      advance(60),
      click("cp-trigger-b"),
      advance(60),
      check("cp-dialog-b", "exists", true),
      check("cp-input-b", "focused", true),
      check("cp-active-id", "text", "scope-b-only"),
      capture("scope-b-open"),

      key("Enter"),
      advance(300),
      check("cp-dialog-b", "exists", false),
      check("cp-trigger-b", "focused", true),
      check("cp-log", "text", "executions:1:scope-b-only"),
    ]);

    const scopeBTree = await readTree(out, "scope-b-open");
    const scopeBList = inspectList(scopeBTree, "cp-list-b");
    assert.equal(scopeBList.count, 1, `expected only Scope B's 1 command, got ${scopeBList.count}`);
    assert.ok(
      scopeBList.joinedText.includes("Scope B Isolated Command"),
      `expected Scope B command, got ${scopeBList.joinedText}`,
    );
    assert.ok(
      !scopeBList.joinedText.includes("Alpha Workspace Overview"),
      `Scope A command leaked into Scope B: ${scopeBList.joinedText}`,
    );
    assert.equal(result.teardown?.clean, true);
  },
);
