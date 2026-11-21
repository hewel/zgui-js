import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

// Valid 1x1 red PNG (67 bytes).
const PNG_1X1_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

const gestureClipboardSource = `
import { Button, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  clipboardReadText,
  readClipboardImage,
  readClipboardText,
  writeClipboardText,
} from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [bgText, setBgText] = createSignal("PENDING");
    const [bgImg, setBgImg] = createSignal("PENDING");
    const [gestureText, setGestureText] = createSignal("NONE");
    const [writeStatus, setWriteStatus] = createSignal("IDLE");

    try {
      const req = readClipboardText();
      req.cancel();
      setBgText("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setBgText(\`DENIED:\${code}\`);
    }

    try {
      const req = readClipboardImage();
      req.cancel();
      setBgImg("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setBgImg(\`DENIED:\${code}\`);
    }

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 8px">
        <text testId="bg-text">{bgText()}</text>
        <text testId="bg-img">{bgImg()}</text>
        <text testId="gesture-text">{gestureText()}</text>
        <text testId="write-status">{writeStatus()}</text>
        <row style="gap: 8px">
          <Button
            testId="btn-read"
            onClick={() => {
              void clipboardReadText().then(val => setGestureText(val ?? "NULL"));
            }}
          >
            Read
          </Button>
          <Button
            testId="btn-write-secret"
            onClick={() => {
              void writeClipboardText("top-secret-token", { secret: true }).promise.then(() => {
                setWriteStatus("WROTE_SECRET");
              });
            }}
          >
            Write Secret
          </Button>
        </row>
      </column>
    );
  }, root);
}
`;

const imageClipboardSource = `
import { Button, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  clipboardReadImage,
  readClipboardImage,
  writeClipboardImage,
} from "@hewel/zgui-services";
import type { ImageHandle } from "@hewel/zgui-services";

export const hostApiVersion = 1;

const PNG_BYTES = new Uint8Array([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
  0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
  0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
  0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
  0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
]);

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [imgSrc, setImgSrc] = createSignal<string | null>(null);
    const [status, setStatus] = createSignal("LOADING");
    let secondHandle: ImageHandle | null = null;
    onCleanup(() => {
      secondHandle?.release();
      secondHandle = null;
    });

    void clipboardReadImage().then(async handle => {
      if (!handle) {
        setStatus("EMPTY");
        return;
      }
      setImgSrc(handle.src);
      await writeClipboardImage(handle, { secret: true }).promise;
      setStatus("REWRITTEN_HANDLE");
    });

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 8px">
        <text testId="img-status">{status()}</text>
        <text testId="img-src">{imgSrc() ?? "NONE"}</text>
        <Show when={imgSrc()}>
          {src => <image testId="clip-preview" src={src()} style="width: 32px; height: 32px" />}
        </Show>
        <Button
          testId="btn-roundtrip-bytes"
          onClick={() => {
            void writeClipboardImage(PNG_BYTES, { secret: false }).promise.then(async () => {
              const next = await readClipboardImage().promise;
              secondHandle?.release();
              secondHandle = next;
              setStatus(next?.src.startsWith("zgui-blob://") ? "ROUNDTRIP_OK" : "ROUNDTRIP_BAD");
            });
          }}
        >
          Roundtrip PNG Bytes
        </Button>
      </column>
    );
  }, root);
}
`;

const leakClipboardImageSource = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { readClipboardImage } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [status, setStatus] = createSignal("WAITING");
    void readClipboardImage().promise.then(handle => {
      // Intentionally omit handle.release() to verify teardown catches leaked ImageHandles.
      setStatus(handle ? "LEAKED" : "EMPTY");
    });
    return (
      <column style="width: 100%; height: 100%; padding: 12px">
        <text testId="leak-status">{status()}</text>
      </column>
    );
  }, root);
}
`;

test("clipboard denies non-gesture reads without capability, allows gesture reads, and propagates secret writes (#59)", async t => {
  const temp = await createTempWorkspace("preview-clipboard-gesture-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "clip-gesture",
    gestureClipboardSource,
    { rawSource: true },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "clipboard-gesture",
    services: {
      clipboard: { text: "seeded-hello" },
    },
    events: [],
  });

  const click = (id, testId) => ({
    id,
    op: "pointer.click",
    button: "primary",
    target: { surface: "main", testId },
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "clipboard-gesture",
    requiredCapabilities: [],
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-initial", op: "capture", captureId: "initial", surface: "main" },
      click("click-read-1", "btn-read"),
      { id: "settle-1", op: "clock.advance", byMs: 32 },
      { id: "cap-after-read", op: "capture", captureId: "after-gesture-read", surface: "main" },
      click("click-write-secret", "btn-write-secret"),
      { id: "settle-2", op: "clock.advance", byMs: 32 },
      click("click-read-2", "btn-read"),
      { id: "settle-3", op: "clock.advance", byMs: 32 },
      { id: "cap-after-secret", op: "capture", captureId: "after-secret-write", surface: "main" },
    ],
  });

  const initialTree = await readTree(out, "initial");
  assert.equal(getNode(initialTree, "bg-text")?.text, "DENIED:E_CAPABILITY");
  assert.equal(getNode(initialTree, "bg-img")?.text, "DENIED:E_CAPABILITY");

  const readTree1 = await readTree(out, "after-gesture-read");
  assert.equal(getNode(readTree1, "gesture-text")?.text, "seeded-hello");

  const finalTree = await readTree(out, "after-secret-write");
  assert.equal(getNode(finalTree, "write-status")?.text, "WROTE_SECRET");
  assert.equal(getNode(finalTree, "gesture-text")?.text, "top-secret-token");

  const services = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
  const clipRecord = services.records.find(r => r.kind === "clipboard");
  assert.ok(clipRecord, "expected clipboard record in services.json");
  assert.equal(clipRecord.detail.format, "text");
  assert.equal(clipRecord.detail.text, "top-secret-token");
  assert.equal(clipRecord.detail.excludeFromHistory, true);
  assert.equal(result.teardown.clean, true);
});

test("clipboard image roundtrip uses ImageHandle outside JS heap with clipboard.read capability and clean teardown (#59)", async t => {
  const temp = await createTempWorkspace("preview-clipboard-image-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "clip-image",
    imageClipboardSource,
    { rawSource: true, capabilities: { clipboard: { read: true } } },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "clipboard-image",
    services: {
      clipboard: { imageBase64: PNG_1X1_BASE64 },
    },
    events: [],
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "clipboard-image",
    requiredCapabilities: [],
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-initial", op: "capture", captureId: "initial", surface: "main" },
      {
        id: "click-roundtrip",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-roundtrip-bytes" },
      },
      { id: "settle-1", op: "clock.advance", byMs: 32 },
      { id: "cap-after", op: "capture", captureId: "after-roundtrip", surface: "main" },
    ],
  });

  const initial = await readTree(out, "initial");
  assert.equal(getNode(initial, "img-status")?.text, "REWRITTEN_HANDLE");
  assert.match(getNode(initial, "img-src")?.text ?? "", /^zgui-blob:\/\//);

  const after = await readTree(out, "after-roundtrip");
  assert.equal(getNode(after, "img-status")?.text, "ROUNDTRIP_OK");

  const services = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
  const clipRecord = services.records.find(r => r.kind === "clipboard");
  assert.ok(clipRecord, "expected clipboard record in services.json");
  assert.equal(clipRecord.detail.format, "image");
  assert.equal(clipRecord.detail.mediaType, "image/png");
  assert.equal(clipRecord.detail.byteLength, 70);
  assert.equal(clipRecord.detail.excludeFromHistory, false);
  assert.equal(result.teardown.clean, true);
});

test("unreleased clipboard ImageHandle fails teardown with E_TEARDOWN (#59)", async t => {
  const temp = await createTempWorkspace("preview-clipboard-leak-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "clip-leak",
    leakClipboardImageSource,
    { rawSource: true, capabilities: { clipboard: { read: true } } },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "clipboard-leak",
    services: {
      clipboard: { imageBase64: PNG_1X1_BASE64 },
    },
    events: [],
  });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "clipboard-leak",
    requiredCapabilities: [],
    allowFailure: true,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-leaked", op: "capture", captureId: "leaked", surface: "main" },
    ],
  });

  assert.equal(result.exitCode, 9);
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.clean, false);
  assert.equal(result.teardown.resources.imageHandles, 1);
});
