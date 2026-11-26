import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

const source = `
import { Image, mountStylesheet, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { registerImageBytes } from "@hewel/zgui-services";

export const hostApiVersion = 1;

function utf8Bytes(text: string): Uint8Array {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) bytes[i] = text.charCodeAt(i) & 0x7f;
  return bytes;
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #10141c; }");
    const svg = [
      '<svg xmlns="http://www.w3.org/2000/svg" width="220" height="80" viewBox="0 0 220 80">',
      '  <rect x="0" y="0" width="220" height="80" fill="#0b1220"/>',
      '  <rect x="8" y="8" width="20" height="20" fill="#ff3b30"/>',
      '  <text x="40" y="54" font-family="sans-serif" font-size="32" fill="#00ff88">HELLO</text>',
      '</svg>',
    ].join("");
    const handle = registerImageBytes(utf8Bytes(svg));
    onCleanup(() => handle.release());
    return (
      <box style={{ width: "240px", height: "100px", padding: "10px", backgroundColor: "#10141c" }}>
        <Image testId="svg-diagram" src={handle.src} style={{ width: "220px", height: "80px" }} />
      </box>
    );
  }, root);
}
`;

test("SVG <text> inside registerImageBytes renders visible glyph pixels in deterministic preview (#52)", async t => {
  const temp = await createTempWorkspace("preview-svg-image-text-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "svg-image-text", source, {
    rawSource: true,
  });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "svg-image-text",
    requiredCapabilities: [],
    width: 240,
    height: 100,
    scale: 1,
    steps: [
      { id: "wait-svg", op: "clock.advance", byMs: 50 },
      { id: "capture-svg", op: "capture", captureId: "initial" },
    ],
  });

  const tree = await readTree(out, "initial");
  const img = getNode(tree, "svg-diagram");
  const png = await readPng(path.join(out, "captures/initial.png"));
  const ox = Math.round(img.boundsDevice.x);
  const oy = Math.round(img.boundsDevice.y);

  // 1. Verify vector <rect> shape rendered red at (18, 18) inside the SVG.
  const rectPx = pixelAt(png, ox + 18, oy + 18);
  assert.ok(
    rectPx[0] > 200 && rectPx[1] < 90 && rectPx[2] < 90,
    `expected red <rect> pixel, got rgba(${rectPx.join(", ")})`,
  );

  // 2. Scan the <text> bounding region (x in [40..180], y in [22..60]) for bright green glyph pixels (#00ff88).
  let greenGlyphPixels = 0;
  for (let y = 22; y <= 60; y += 1) {
    for (let x = 40; x <= 180; x += 1) {
      const px = pixelAt(png, ox + x, oy + y);
      if (px[1] > 170 && px[0] < 60) greenGlyphPixels += 1;
    }
  }
  assert.ok(
    greenGlyphPixels > 60,
    `expected SVG <text> to render visible green glyph pixels inside registerImageBytes image, got ${greenGlyphPixels}`,
  );
});

const hidpiVectorSource = `
import { Image, mountStylesheet, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { registerImageBytes } from "@hewel/zgui-services";

export const hostApiVersion = 1;

function asciiBytes(text: string): Uint8Array {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) bytes[i] = text.charCodeAt(i) & 0x7f;
  return bytes;
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #000000; }");
    // Intrinsic viewBox is 60x24, displayed at 180x72 CSS px at scale=2 (360x144 device px = 6x).
    // A 1-unit green bar at x=20 spans exact device x=[120..126) with no blurred bleed.
    const svg = [
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 24">',
      '  <rect x="0" y="0" width="60" height="24" fill="#000000"/>',
      '  <rect x="20" y="2" width="1" height="20" fill="#00ff00"/>',
      '  <text x="26" y="17" font-family="sans-serif" font-size="12" fill="#ffffff">HiDPI</text>',
      '</svg>',
    ].join("");
    const handle = registerImageBytes(asciiBytes(svg));
    onCleanup(() => handle.release());
    return (
      <box style={{ width: "200px", height: "90px", padding: "8px", backgroundColor: "#000000" }}>
        <Image testId="svg-hidpi" src={handle.src} style={{ width: "180px", height: "72px" }} />
      </box>
    );
  }, root);
}
`;

test("SVG diagrams rasterize crisply above native viewBox extent at scale=2", async t => {
  const temp = await createTempWorkspace("preview-svg-hidpi-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "svg-hidpi", hidpiVectorSource, {
    rawSource: true,
  });
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "svg-hidpi",
    requiredCapabilities: [],
    width: 200,
    height: 90,
    scale: 2,
    steps: [
      { id: "wait-svg", op: "clock.advance", byMs: 50 },
      { id: "capture-svg", op: "capture", captureId: "hidpi" },
    ],
  });

  const tree = await readTree(out, "hidpi");
  const img = getNode(tree, "svg-hidpi");
  assert.equal(img.boundsLogical.width, 180);
  assert.equal(img.boundsLogical.height, 72);
  assert.equal(img.boundsDevice.width, 360);
  assert.equal(img.boundsDevice.height, 144);

  const png = await readPng(path.join(out, "captures/hidpi.png"));
  const ox = Math.round(img.boundsDevice.x);
  const oy = Math.round(img.boundsDevice.y);
  const midY = oy + 72;

  for (const dx of [120, 121, 122, 123, 124, 125]) {
    const px = pixelAt(png, ox + dx, midY);
    assert.ok(
      px[0] === 0 && px[1] === 255 && px[2] === 0,
      `expected crisp pure green bar texel at ox+${dx}, got rgba(${px.join(", ")})`,
    );
  }
  for (const dx of [118, 119, 126, 127]) {
    const px = pixelAt(png, ox + dx, midY);
    assert.ok(
      px[0] === 0 && px[1] === 0 && px[2] === 0,
      `expected crisp dark background adjacent to bar at ox+${dx} without bilinear bleed, got rgba(${px.join(", ")})`,
    );
  }

  assert.equal(result.teardown?.clean, true);
});
