import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
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

const svg = color =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="${color}"/></svg>`;
const capture = { id: "image", op: "capture", captureId: "image" };
const blobAsset = "assets/input.svg";
const redPng = Array.from(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAIElEQVR4nGP4z8BAFUQdU0YNGjVo1KBRg0YNGjWIIgQABMo93z5bbNQAAAAASUVORK5CYII=",
    "base64",
  ),
);

function appSource({ file, blob = false, leak = false } = {}) {
  return `
    import { Image, renderApp, type NativeNode } from "@hewel/zgui";
    import { ${leak ? "registerImageAsset" : "loadImageAsset"} } from "@hewel/zgui-services";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        const style = { width: "24px", height: "24px" };
        ${blob ? `const src = ${leak ? "registerImageAsset" : "loadImageAsset"}(${JSON.stringify(blobAsset)}).src;` : ""}
        return <column>
          ${file ? `<Image testId="file" src={${JSON.stringify(file)}} style={style} />` : ""}
          ${blob ? `<Image testId="blob" src={src} style={style} />` : ""}
        </column>;
      }, root);
    }
  `;
}

async function setup(t) {
  const temp = await createTempWorkspace("preview-capability-images-");
  t.after(() => temp.cleanup());
  const prefix = path.join(temp.dir, "external");
  const filename = path.join(prefix, "disk image.svg");
  await mkdir(prefix);
  await writeFile(filename, svg("#ff0000"));
  const file = pathToFileURL(filename).href;
  const assets = {
    "input.svg": svg("#ff0000"),
    "fixture-file.svg": svg("#00ff00"),
    "fixture-blob.svg": svg("#0000ff"),
  };

  return {
    file,
    async build(name, source, capabilities) {
      return buildFixtureApp(path.join(temp.dir, name), name, source, {
        rawSource: true,
        assets,
        capabilities,
      });
    },
    async run(name, bundle, images, allowFailure = false, runtimeEnvironment) {
      const fixture = path.join(temp.dir, `${name}-fixture.json`);
      await writeJson(fixture, {
        schemaVersion: 1,
        id: name,
        services: { images },
        events: [],
        runtimeEnvironment,
      });
      return runScenario({
        projectDir: temp.dir,
        name,
        bundle,
        fixture,
        allowFailure,
        requiredCapabilities: ["pixels", "assets.native-readiness"],
        steps: [capture],
      });
    },
    capabilities: { imagePrefixes: [prefix] },
  };
}

test(
  "preview draws deterministic file and blob bytes, never ambient files",
  { timeout: 30000 },
  async t => {
    const { build, run, file, capabilities } = await setup(t);
    const bundle = await build("dynamic-images", appSource({ file, blob: true }), capabilities);
    const images = {
      files: { [file]: "assets/fixture-file.svg" },
      blobs: { [blobAsset]: "assets/fixture-blob.svg" },
    };
    const { out } = await run("fixture-images", bundle, images);
    const tree = await readTree(out, "image");
    const png = await readPng(path.join(out, "captures/image.png"));
    for (const [id, expected] of [
      ["file", [0, 255, 0, 255]],
      ["blob", [0, 0, 255, 255]],
    ]) {
      const { x, y, width, height } = getNode(tree, id).boundsDevice;
      assert.deepEqual(
        pixelAt(png, Math.floor(x + width / 2), Math.floor(y + height / 2)),
        expected,
        id,
      );
    }
    const absentFile = await run("missing-file-fixture", bundle, { blobs: images.blobs }, true);
    assert.equal(absentFile.result.firstError?.code, "E_ASSET_FAILED");
    const absentBlob = await run("missing-blob-fixture", bundle, { files: images.files }, true);
    assert.equal(absentBlob.result.firstError?.code, "E_FIXTURE_CONTRACT");

    const noAuthority = await build("no-image-authority", appSource({ file }));
    const denied = await run("denied-file", noAuthority, { files: images.files }, true);
    assert.equal(denied.result.firstError?.code, "E_ASSET_FAILED");
  },
);

test("preview images use the resolved runtime template without a host directory", async t => {
  const { build, run } = await setup(t);
  const file = "file:///zgui-preview-only/runtime/z-shell/media/sample.svg";
  const bundle = await build("portable-images", appSource({ file }), {
    imagePrefixes: ["$XDG_RUNTIME_DIR/z-shell/media"],
  });
  const images = { files: { [file]: "assets/fixture-file.svg" } };
  const { out } = await run("resolved-image", bundle, images, false, {
    xdgRuntimeDir: "/zgui-preview-only/runtime",
  });
  const png = await readPng(path.join(out, "captures/image.png"));
  const { x, y, width, height } = getNode(await readTree(out, "image"), "file").boundsDevice;
  assert.deepEqual(
    pixelAt(png, Math.floor(x + width / 2), Math.floor(y + height / 2)),
    [0, 255, 0, 255],
  );
  const { result } = await run("missing-runtime-root", bundle, images, true);
  assert.equal(result.firstError?.code, "E_CAPABILITY");
  assert.match(result.firstError?.message, /XDG_RUNTIME_DIR/);
});

test("preview reports leaked Image handles as teardown failures", async t => {
  const { build, run } = await setup(t);
  const bundle = await build("leaked-image", appSource({ blob: true, leak: true }));
  const { result } = await run(
    "leak",
    bundle,
    { blobs: { [blobAsset]: "assets/fixture-blob.svg" } },
    true,
  );
  assert.equal(result.captureCount, 1);
  assert.equal(result.exitCode, 9);
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.resources.imageHandles, 1);
});

test("preview draws caller-supplied PNG and SVG without image fixtures and reports direct-byte leaks", async t => {
  const { build, run } = await setup(t);
  for (const leak of [false, true]) {
    const name = leak ? "leaked-bytes" : "scoped-bytes";
    const bundle = await build(
      name,
      `
      import { Image, renderApp, type NativeNode } from "@hewel/zgui";
      import { loadImageBytes, registerImageBytes } from "@hewel/zgui-services";
      export const hostApiVersion = 1;
      export function mount(root: NativeNode) {
        return renderApp(() => {
          for (const [bytes, expected] of [
            [[1], TypeError],
            [new Uint8Array(), RangeError],
            [new Uint8Array(8 * 1024 * 1024 + 1), RangeError],
          ] as const) {
            try {
              registerImageBytes(bytes as Uint8Array);
              throw new Error("invalid image was registered");
            } catch (error) {
              if (!(error instanceof expected)) throw error;
            }
          }
          const payload = new Uint8Array(${redPng.length + 2});
          payload.set(${JSON.stringify(redPng)}, 1);
          const png = ${leak ? "registerImageBytes" : "loadImageBytes"}(payload.subarray(1, -1));
          payload.fill(0);
          const svg = loadImageBytes(Uint8Array.from(
            Array.from(${JSON.stringify(svg("#0000ff"))}, char => char.charCodeAt(0)),
          ));
          if (!png.src.startsWith("zgui-blob://") || !svg.src.startsWith("zgui-blob://") || png.src === svg.src) {
            throw new Error("expected distinct opaque image handles");
          }
          const style = { width: "24px", height: "24px" };
          return <column><Image testId="png" src={png.src} style={style} />
            <Image testId="svg" src={svg.src} style={style} /></column>;
        }, root);
      }
    `,
    );
    const { out, result } = await run(name, bundle, undefined, leak);
    assert.equal(result.captureCount, 1);
    const tree = await readTree(out, "image");
    const image = await readPng(path.join(out, "captures/image.png"));
    for (const [id, rgba] of [
      ["png", [255, 0, 0, 255]],
      ["svg", [0, 0, 255, 255]],
    ]) {
      const { x, y, width, height } = getNode(tree, id).boundsDevice;
      assert.deepEqual(
        pixelAt(image, Math.floor(x + width / 2), Math.floor(y + height / 2)),
        rgba,
        id,
      );
    }
    assert.equal(result.firstError?.code, leak ? "E_TEARDOWN" : undefined);
    assert.equal(result.teardown.resources.imageHandles ?? 0, Number(leak));
  }
});
