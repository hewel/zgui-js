import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readFrame,
  readPng,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const endpoint = "/zgui-preview-only/frames.sock";
const blue = {
  streamId: 1,
  frameId: 2,
  width: 16,
  height: 8,
  pixelFormat: "nv12",
  pattern: { solid: "#0000ff" },
};

function frame(atMs, frameId, pixelFormat, width, height, pattern) {
  return {
    kind: "frame",
    atMs,
    streamId: 1,
    frameId,
    width,
    height,
    pixelFormat,
    pattern,
  };
}

test("frame streams fit and clip real pixels, survive resize, and retire on surface recreation", async t => {
  const { dir, cleanup } = await createTempWorkspace("preview-frame-stream-");
  t.after(cleanup);
  const name = "frame-stream";
  const bundle = await buildFixtureApp(
    path.join(dir, name),
    name,
    `
    import { Image, createSignal, renderApp, type NativeNode, type FrameStreamHandle } from "@hewel/zgui";
    import { connectUnixSocket } from "@hewel/zgui-services";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        const [stream, setStream] = createSignal<FrameStreamHandle>();
        connectUnixSocket(${JSON.stringify(endpoint)}, {
          framing: "length",
          acceptFrameStreams: true,
          onMessage(message) { if (message === "release") stream()?.release(); },
          onFrameStream(next) {
            if (next.kind !== "frame-stream" || next.streamId !== 1 || next.width <= 0 || next.height <= 0) {
              throw new Error("invalid frame stream handle");
            }
            setStream(next);
          },
        });
        return <column style={{ backgroundColor: "black" }}>
          <Image src={stream()?.src} testId="video" style={{ width: "80px", height: "80px", objectFit: "contain", borderRadius: "24px" }} />
          <image src={stream()?.src} testId="intrinsic" style={{ width: "80px", alignSelf: "start" }} />
          <box style={{ width: "40px", height: "80px", overflow: "hidden" }}>
            <Image src={stream()?.src} testId="cover" style={{ width: "80px", height: "80px", objectFit: "cover", objectPosition: "left center", clipPath: "polygon(0% 0%, 100% 0%, 0% 100%)" }} />
          </box>
        </column>;
      }, root);
    }
  `,
    { rawSource: true, capabilities: { unixSockets: [endpoint] } },
  );
  const socket = { path: endpoint, framing: "length", send: [] };
  const fixture = path.join(dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: name,
    services: {
      sockets: [
        {
          ...socket,
          events: [
            frame(1, 1, "rgba8", 16, 8, {
              quadrants: ["#ff0000", "#ffff00", "#ff0000", "#ffff00"],
            }),
            frame(201, 3, "rgba8", 8, 8, { solid: "#00ff00" }),
            { atMs: 301, kind: "message", text: "release" },
            frame(401, 4, "rgba8", 8, 8, { solid: "#00ff00" }),
          ],
        },
        { ...socket, events: [] },
      ],
    },
    events: [],
  });
  const advance = (id, byMs) => ({ id, op: "clock.advance", byMs });
  const capture = id => ({ id, op: "capture", captureId: id });
  const { out, result } = await runScenario({
    projectDir: dir,
    name,
    bundle,
    fixture,
    requiredCapabilities: ["pixels", "services.fixtures", "surface.recreate"],
    steps: [
      advance("red-time", 50),
      capture("red"),
      { id: "blue-submit", op: "service.emit", channel: "frame-stream", value: blue },
      advance("blue-time", 100),
      capture("blue"),
      advance("green-time", 100),
      capture("green"),
      advance("release-time", 100),
      capture("released"),
      advance("last-time", 100),
      capture("last"),
      { id: "recreate", op: "surface.recreate", surface: "main" },
      capture("recreated"),
    ],
  });
  /** @type {readonly (readonly [string, readonly number[], boolean])[]} */
  const captures = [
    ["red", [255, 0, 0, 255], true],
    ["blue", [0, 0, 255, 255], true],
    ["green", [0, 255, 0, 255], false],
    ["last", [0, 255, 0, 255], false],
    ["released", [0, 0, 0, 255], true],
    ["recreated", [0, 0, 0, 255], true],
  ];
  for (const [id, rgba, letterbox] of captures) {
    const tree = await readTree(out, id);
    const png = await readPng(path.join(out, "captures", `${id}.png`));
    const { x, y, width, height } = getNode(tree, "video").boundsDevice;
    const sample = (u, v) => pixelAt(png, Math.floor(x + width * u), Math.floor(y + height * v));
    assert.deepEqual(sample(0.25, 0.5), rgba, id);
    assert.deepEqual(sample(0.02, 0.02), [0, 0, 0, 255], `${id}: rounded corner`);
    assert.deepEqual(sample(0.5, 0.1), letterbox ? [0, 0, 0, 255] : rgba, `${id}: fit`);
    if (["red", "blue", "green"].includes(id)) {
      const intrinsic = getNode(tree, "intrinsic").boundsDevice;
      assert.equal(intrinsic.width / intrinsic.height, id === "green" ? 1 : 2, id);
    }
    if (id === "red") {
      const cover = getNode(tree, "cover").boundsDevice;
      const at = (u, v) =>
        pixelAt(png, Math.floor(cover.x + cover.width * u), Math.floor(cover.y + cover.height * v));
      assert.deepEqual(at(0.1, 0.1), [255, 0, 0, 255], "cover fills rather than letterboxing");
      assert.deepEqual(at(0.4, 0.8), [0, 0, 0, 255], "clip-path cuts through the external image");
      assert.deepEqual(at(0.8, 0.5), [0, 0, 0, 255], "ancestor overflow clips the covered image");
    }
  }
  const [first, unchanged, resized] = await Promise.all(
    ["red", "blue", "green"].map(id => readFrame(out, id)),
  );
  assert.equal(unchanged.stylePasses, first.stylePasses, "new pixels must not run the cascade");
  assert.equal(unchanged.relayouts, first.relayouts, "same dimensions skip box measurement");
  assert.ok(resized.relayouts > unchanged.relayouts, "intrinsic aspect ratio invalidates layout");
  assert.equal(result.teardown.clean, true);
  assert.equal(result.teardown.resources.frameStreams, 0);
  assert.equal(result.teardown.resources.importedFrameFds, 0);
});
