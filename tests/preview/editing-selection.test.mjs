import assert from "node:assert/strict";
import { test } from "node:test";
import path from "node:path";
import { buildFixtureApp, createTempWorkspace, runScenario } from "./harness.mjs";

const selectionPreviewSource = `
import {
  Button,
  TextArea,
  createSignal,
  flush,
  focus,
  getSelection,
  renderApp,
  selectAll,
  setSelection,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [text, setText] = createSignal("A中🙂B");
    const [lastEventSel, setLastEventSel] = createSignal("none");
    const [polledSel, setPolledSel] = createSignal("none");
    const [editorEl, setEditorEl] = createSignal<NativeNode | null>(null);

    const readNow = () => {
      const sel = getSelection(editorEl);
      setPolledSel(sel ? \`\${sel.start}:\${sel.end}:\${text().slice(sel.start, sel.end)}\` : "null");
    };

    const wrapBold = () => {
      const value = text();
      const sel = getSelection(editorEl) ?? { start: 0, end: 0 };
      const chosen = value.slice(sel.start, sel.end);
      const next = \`\${value.slice(0, sel.start)}**\${chosen}**\${value.slice(sel.end)}\`;
      const nextStart = sel.start + 2;
      const nextEnd = nextStart + chosen.length;
      setText(next);
      flush();
      setSelection(editorEl, nextStart, nextEnd);
      readNow();
    };

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 8px">
        <row style="gap: 8px">
          <Button testId="btn-select-cjk-emoji" onClick={() => { setSelection(editorEl, 1, 4); readNow(); }}>
            Select CJK+Emoji
          </Button>
          <Button testId="btn-clamp-surrogate" onClick={() => { setSelection(editorEl, 3); readNow(); }}>
            Clamp Surrogate
          </Button>
          <Button testId="btn-wrap-bold" onClick={wrapBold}>
            Bold
          </Button>
          <Button testId="btn-select-all" onClick={() => { const el = editorEl(); if (el) { selectAll(el); focus(el); } readNow(); }}>
            Select All
          </Button>
          <Button testId="btn-read" onClick={readNow}>
            Read
          </Button>
        </row>
        <TextArea
          ref={setEditorEl}
          testId="editor"
          style="width: 480px; height: 96px; border: 1px solid #888; padding: 6px"
          value={text()}
          onInput={event => {
            setText(event.value);
            setLastEventSel(\`\${event.selection.start}:\${event.selection.end}\`);
          }}
        />
        <text testId="value-out">{text()}</text>
        <text testId="event-sel">{lastEventSel()}</text>
        <text testId="polled-sel">{polledSel()}</text>
      </column>
    );
  }, root);
}
`;

test("NativeValueEvent.selection and getSelection/setSelection/selectAll use UTF-16 offsets across ASCII, CJK, and emoji (#56)", async t => {
  const temp = await createTempWorkspace("preview-utf16-selection-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "utf16-selection",
    selectionPreviewSource,
    {
      rawSource: true,
    },
  );

  const check = (id, testId, equals) => ({
    id,
    op: "assert",
    target: { surface: "main", testId },
    check: { property: "text", equals },
  });
  const click = (id, testId) => ({
    id,
    op: "pointer.click",
    button: "primary",
    target: { surface: "main", testId },
  });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "utf16-selection",
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.text",
      "tree.inspect",
    ],
    width: 640,
    height: 300,
    scale: 1,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      // Select "中🙂" at UTF-16 1..4 inside "A中🙂B"
      click("select-cjk-emoji", "btn-select-cjk-emoji"),
      check("check-cjk-emoji", "polled-sel", "1:4:中🙂"),
      // Wrap selected "中🙂" with "**...**" -> "A**中🙂**B", selection stays on "中🙂" (UTF-16 3..6)
      click("wrap-bold", "btn-wrap-bold"),
      check("check-wrapped-value", "value-out", "A**中🙂**B"),
      check("check-wrapped-sel", "polled-sel", "3:6:中🙂"),
      // Let Command::SetValue + Command::Select settle on the native editor model, then re-read
      { id: "settle-wrap", op: "clock.advance", byMs: 32 },
      click("read-settled", "btn-read"),
      check("check-settled-sel", "polled-sel", "3:6:中🙂"),
      // Select all + focus editor, then replace entire selection via IME commit with CJK + emoji "你好🎉"
      click("reselect-all", "btn-select-all"),
      check("check-select-all", "polled-sel", "0:9:A**中🙂**B"),
      { id: "settle-all", op: "clock.advance", byMs: 32 },
      // Replace entire selection with CJK + emoji "你好🎉" (UTF-16 len 4: 你=1, 好=1, 🎉=2)
      { id: "commit-cjk-emoji", op: "text.commit", surface: "main", text: "你好🎉" },
      check("check-committed-value", "value-out", "你好🎉"),
      check("check-committed-event-sel", "event-sel", "4:4"),
      // Mid-surrogate clamp on "你好🎉" (index 3 splits 🎉 at UTF-16 2..4 -> clamps to 2:2)
      click("clamp-surrogate", "btn-clamp-surrogate"),
      check("check-clamped-sel", "polled-sel", "2:2:"),
      { id: "cap-final", op: "capture", captureId: "final" },
    ],
  });

  assert.equal(result.teardown?.clean, true);
});
