import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const selectionFixture = `
import {
  TextArea,
  TextField,
  createSignal,
  getSelection,
  renderApp,
  selectAll,
  setSelection,
} from "@hewel/zgui";

const [fieldRef, setFieldRef] = createSignal(null);
const [areaRef, setAreaRef] = createSignal(null);
const [boxRef, setBoxRef] = createSignal(null);
const [missingRef] = createSignal(null);

export const readField = () => getSelection(fieldRef);
export const readArea = () => getSelection(areaRef);
export const readMissing = () => getSelection(missingRef);
export const readBox = () => getSelection(boxRef);
export const setFieldSel = (start, end) => (end === undefined ? setSelection(fieldRef, start) : setSelection(fieldRef, start, end));
export const setAreaSel = (start, end) => (end === undefined ? setSelection(areaRef, start) : setSelection(areaRef, start, end));
export const setBoxSel = () => setSelection(boxRef, 0, 1);
export const setMissingSel = () => setSelection(missingRef, 0, 1);
export const selectAllField = () => selectAll(fieldRef);
export const selectAllArea = () => selectAll(areaRef);
export const selectAllBox = () => selectAll(boxRef);
export const selectAllMissing = () => selectAll(missingRef);

export function mount(root) {
  return renderApp(() => (
    <column>
      <TextField ref={setFieldRef} testId="field" value={"A中🙂B"} />
      <TextArea ref={setAreaRef} testId="area" value={"第一行\\n🚀end"} />
      <box ref={setBoxRef} testId="plain-box" />
    </column>
  ), root);
}
`;

test("getSelection, setSelection, and selectAll handle UTF-16 ranges, null accessors, and reject non-editable nodes (#56)", async t => {
  const h = createHost();
  const mod = await moduleFor(t, selectionFixture);
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));

  assert.equal(mod.readField(), null);
  assert.equal(mod.readMissing(), null);
  assert.doesNotThrow(() => h.runTurn(() => mod.setMissingSel()));
  assert.doesNotThrow(() => h.runTurn(() => mod.selectAllMissing()));

  // ASCII + CJK + emoji UTF-16 indexing: "A中🙂B" has length 5 ("中🙂" is 1..4)
  h.runTurn(() => mod.setFieldSel(1, 4));
  assert.deepEqual(mod.readField(), { start: 1, end: 4 });
  assert.equal("A中🙂B".slice(1, 4), "中🙂");

  // Default end = start for caret placement + surrogate pair boundary clamping (index 3 splits 🙂)
  h.runTurn(() => mod.setFieldSel(3));
  assert.deepEqual(mod.readField(), { start: 2, end: 2 });

  h.runTurn(() => mod.selectAllField());
  assert.deepEqual(mod.readField(), { start: 0, end: 5 });

  // Multiline editor with CJK + newline + emoji: "第一行\n🚀end" has UTF-16 length 9
  h.runTurn(() => mod.setAreaSel(4, 6));
  assert.deepEqual(mod.readArea(), { start: 4, end: 6 });
  assert.equal("第一行\n🚀end".slice(4, 6), "🚀");

  h.runTurn(() => mod.selectAllArea());
  assert.deepEqual(mod.readArea(), { start: 0, end: 9 });

  // Non-editable nodes and non-finite offsets throw cleanly
  assert.throws(() => mod.readBox(), /field or editor/);
  assert.throws(() => h.runTurn(() => mod.setBoxSel()), /field or editor/);
  assert.throws(() => h.runTurn(() => mod.selectAllBox()), /field or editor/);
  assert.throws(() => h.runTurn(() => mod.setFieldSel(Number.NaN, 1)), RangeError);
  assert.throws(() => h.runTurn(() => mod.setFieldSel(0, Number.POSITIVE_INFINITY)), RangeError);
});
