import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const APP_SOURCE = `
import {
  Button,
  Show,
  TextField,
  clearDocumentSelection,
  createSignal,
  getDocumentSelection,
  renderApp,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { readClipboardText } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const [showHeading, setShowHeading] = createSignal(true);
    const [selSummary, setSelSummary] = createSignal("none");
    const [clipText, setClipText] = createSignal("empty");

    const inspectSelection = () => {
      const sel = getDocumentSelection();
      if (!sel) {
        setSelSummary("none");
        return;
      }
      setSelSummary(JSON.stringify({
        anchorOffset: sel.anchor.offset,
        focusOffset: sel.focus.offset,
        isCollapsed: sel.isCollapsed,
        text: sel.text,
      }));
    };

    const readClip = async () => {
      const value = await readClipboardText().promise;
      setClipText(value ?? "<null>");
    };

    return (
      <column
        testId="page"
        style={{
          width: "100%",
          height: "100%",
          padding: "16px",
          gap: "10px",
          backgroundColor: "#ffffff",
          color: "#111111",
        }}
      >
        <row style={{ gap: "6px", flexWrap: "wrap", userSelect: "none" }}>
          <Button testId="btn-inspect" onClick={inspectSelection}>Inspect</Button>
          <Button testId="btn-read-clip" onClick={() => void readClip()}>Clip</Button>
          <Button testId="btn-detach" onClick={() => setShowHeading(false)}>Detach</Button>
          <Button testId="btn-clear" onClick={() => { clearDocumentSelection(); inspectSelection(); }}>Clear</Button>
        </row>

        <text testId="sel-readout" style={{ userSelect: "none", fontSize: "11px" }}>{selSummary()}</text>
        <text testId="clip-readout" style={{ userSelect: "none", fontSize: "11px" }}>{clipText()}</text>

        <column
          testId="doc-container"
          documentSelection
          style={{
            width: "100%",
            padding: "10px",
            gap: "8px",
            backgroundColor: "#ffffff",
            color: "#0066cc",
          }}
        >
          <Show when={showHeading()}>
            <text testId="blk-heading" style={{ fontSize: "20px", fontWeight: "700", color: "#0066cc" }}>
              {"标题🚀Title"}
            </text>
          </Show>
          <text testId="blk-para" style={{ fontSize: "14px", color: "#0066cc" }}>
            {"你好世界 alpha beta gamma delta مرحبا end"}
          </text>
          <text testId="blk-noselect" style={{ userSelect: "none", fontSize: "14px", color: "#888888" }}>
            {"UNSELECTABLE_BADGE"}
          </text>
          <TextField testId="blk-field" defaultValue="FIELD_SECRET" style={{ width: "160px", height: "28px" }} />
          <row testId="blk-row" role="row" style={{ gap: "16px" }}>
            <text testId="blk-cell-a" style={{ fontSize: "14px", color: "#0066cc" }}>{"CellA"}</text>
            <text testId="blk-cell-b" style={{ fontSize: "14px", color: "#0066cc" }}>{"CellB🚀"}</text>
          </row>
        </column>
      </column>
    );
  }, root);
}
`;

let temp;
let bundle;
let fixturePath;

before(async () => {
  temp = await createTempWorkspace("preview-doc-select-");
  bundle = await buildFixtureApp(temp.dir, "doc-select-app", APP_SOURCE, {
    rawSource: true,
    capabilities: { clipboard: { read: true } },
  });
  fixturePath = path.join(temp.dir, "fixture.json");
  await writeJson(fixturePath, {
    schemaVersion: 1,
    id: "doc-select-fixture",
    services: {
      clipboard: {},
    },
    events: [],
  });
});

after(async () => {
  await temp?.cleanup();
});

let seq = 0;
const id = () => `s-${++seq}`;
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const capture = captureId => ({ id: id(), op: "capture", captureId, surface: "main" });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function countBandFillPixels(image, rect) {
  const x0 = Math.max(0, Math.floor(rect.x));
  const y0 = Math.max(0, Math.floor(rect.y));
  const x1 = Math.min(image.width, Math.ceil(rect.x + rect.width));
  const y1 = Math.min(image.height, Math.ceil(rect.y + rect.height));
  let matched = 0;
  for (let y = y0; y < y1; y += 1) {
    for (let x = x0; x < x1; x += 1) {
      const [r, g, b] = pixelAt(image, x, y);
      // Exact blended selection band: rgb(0, 102, 204) @ 0.30 over #ffffff -> ~(179, 209, 240).
      if (Math.abs(r - 179) <= 8 && Math.abs(g - 209) <= 8 && Math.abs(b - 240) <= 8) {
        matched += 1;
      }
    }
  }
  return matched;
}

test(
  "reverse cross-block document selection handles CJK/emoji/bidi, UTF-16 offsets, clipboard copy, resize rewrap highlights, and detach cleanup (#68)",
  { timeout: 60000 },
  async () => {
    const CAPS = [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.keyboard",
      "tree.inspect",
      "surface.resize",
    ];

    // Step 1: Capture initial layout to discover exact bounds of heading and last table cell.
    const probe = await runScenario({
      projectDir: temp.dir,
      bundle,
      fixture: fixturePath,
      name: "probe",
      id: "doc-select-probe",
      width: 520,
      height: 360,
      scale: 1,
      requiredCapabilities: CAPS,
      steps: [advance(32), capture("initial")],
    });
    const tInit = await readTree(probe.out, "initial");
    const headingBox = getNode(tInit, "blk-heading").boundsLogical;
    const cellBBox = getNode(tInit, "blk-cell-b").boundsLogical;

    const startX = cellBBox.x + cellBBox.width + 6;
    const startY = cellBBox.y + cellBBox.height * 0.5;
    const endX = Math.max(1, headingBox.x - 4);
    const endY = headingBox.y + headingBox.height * 0.5;

    // Step 2: Run full reverse drag -> Ctrl+C -> inspect -> resize narrower (rewrap) -> detach endpoint.
    const { out, result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      fixture: fixturePath,
      name: "full",
      id: "doc-select-full",
      width: 520,
      height: 360,
      scale: 1,
      requiredCapabilities: CAPS,
      steps: [
        advance(32),
        capture("before-select"),

        // Reverse drag from right of CellB🚀 up to left of 标题🚀Title
        {
          id: id(),
          op: "pointer.down",
          surface: "main",
          position: { x: startX, y: startY },
          button: "primary",
        },
        {
          id: id(),
          op: "pointer.move",
          surface: "main",
          position: { x: startX, y: (startY + endY) * 0.5 },
        },
        { id: id(), op: "pointer.move", surface: "main", position: { x: endX, y: endY } },
        {
          id: id(),
          op: "pointer.up",
          surface: "main",
          position: { x: endX, y: endY },
          button: "primary",
        },
        advance(32),
        capture("selected-wide"),

        // Copy via Ctrl+C, then inspect JS UTF-16 selection & read clipboard
        {
          id: id(),
          op: "key.press",
          surface: "main",
          key: { character: "c" },
          modifiers: ["Control"],
        },
        advance(32),
        click("btn-inspect"),
        click("btn-read-clip"),
        advance(32),
        capture("after-copy"),

        // Resize surface narrower so paragraph soft-wraps onto multiple lines
        { id: id(), op: "surface.resize", surface: "main", width: 220, height: 420 },
        advance(64),
        click("btn-inspect"),
        advance(32),
        capture("selected-narrow"),

        // Detach heading endpoint -> selection must safely clear to null and remove highlights
        click("btn-detach"),
        advance(32),
        click("btn-inspect"),
        advance(32),
        capture("after-detach"),
      ],
    });

    const expectedText = "标题🚀Title\n你好世界 alpha beta gamma delta مرحبا end\nCellA\tCellB🚀";

    const tCopy = await readTree(out, "after-copy");
    const selJson = JSON.parse(getNode(tCopy, "sel-readout").text);
    // "CellB🚀" has UTF-16 length 5 + 2 = 7 (surrogate pair for 🚀), NOT UTF-8 byte length 9!
    assert.equal(
      selJson.anchorOffset,
      "CellB🚀".length,
      "anchor offset must be UTF-16 code units (7 for CellB🚀)",
    );
    assert.equal(selJson.focusOffset, 0, "reverse drag focus must land at offset 0 of heading");
    assert.equal(selJson.isCollapsed, false);
    assert.equal(selJson.text, expectedText);

    const copiedText = getNode(tCopy, "clip-readout").text;
    assert.equal(copiedText, expectedText, "Ctrl+C must copy exact document-order plain text");

    // Verify painted highlight pixels appear in selected-wide vs before-select!
    const pngBefore = await readPng(path.join(out, "captures/before-select.png"));
    const pngWide = await readPng(path.join(out, "captures/selected-wide.png"));
    const tWide = await readTree(out, "selected-wide");
    const wideParaRect = getNode(tWide, "blk-para").boundsLogical;
    const beforeBand = countBandFillPixels(pngBefore, wideParaRect);
    const wideBand = countBandFillPixels(pngWide, wideParaRect);
    assert.ok(
      wideBand > beforeBand + 800,
      `expected selection band pixels to surge on selected paragraph: before=${beforeBand}, wide=${wideBand}`,
    );

    // Verify narrow resize rewrapped the paragraph, kept selection text identical (no soft-wrap \n),
    // and painted highlight pixels across the newly wrapped lower lines!
    const tNarrow = await readTree(out, "selected-narrow");
    const narrowSel = JSON.parse(getNode(tNarrow, "sel-readout").text);
    assert.equal(narrowSel.text, expectedText, "rewrap must not insert \\n or corrupt selection");
    const narrowParaRect = getNode(tNarrow, "blk-para").boundsLogical;
    assert.ok(
      narrowParaRect.height > wideParaRect.height + 10,
      `expected paragraph to wrap onto more lines when narrowed (${narrowParaRect.height} vs ${wideParaRect.height})`,
    );
    const pngNarrow = await readPng(path.join(out, "captures/selected-narrow.png"));
    const narrowLowerHalf = {
      x: narrowParaRect.x,
      y: narrowParaRect.y + narrowParaRect.height * 0.5,
      width: narrowParaRect.width,
      height: narrowParaRect.height * 0.5,
    };
    const narrowLowerBand = countBandFillPixels(pngNarrow, narrowLowerHalf);

    // Verify detaching endpoint node cleared selection cleanly and removed highlight pixels!
    const tDetach = await readTree(out, "after-detach");
    assert.equal(getNode(tDetach, "sel-readout").text, "none");
    const pngDetach = await readPng(path.join(out, "captures/after-detach.png"));
    const detachParaRect = getNode(tDetach, "blk-para").boundsLogical;
    const detachLowerHalf = {
      x: detachParaRect.x,
      y: detachParaRect.y + detachParaRect.height * 0.5,
      width: detachParaRect.width,
      height: detachParaRect.height * 0.5,
    };
    const detachLowerBand = countBandFillPixels(pngDetach, detachLowerHalf);
    assert.ok(
      narrowLowerBand > detachLowerBand + 400,
      `expected highlight band pixels on wrapped lower lines after resize (selected=${narrowLowerBand} vs detached=${detachLowerBand})`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);
