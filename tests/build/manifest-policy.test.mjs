import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { bundle, publishApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import schema from "../../schemas/package.schema.json" with { type: "json" };
import cases from "../fixtures/capability-conformance.json" with { type: "json" };
import { validateAgainstSchema } from "./schema-fixture.mjs";

const props = schema.properties;
const caps = props.capabilities.properties;
const worker = name => ({
  name,
  path: `workers/${name}.mjs`,
  sourceMap: `workers/${name}.mjs.map`,
});

function checkSchema(value, definition, valid = true) {
  const validate = () => validateAgainstSchema(value, definition, schema);
  if (valid) validate();
  else assert.throws(validate, { name: "AssertionError" }, JSON.stringify(value));
}

async function publisher(t) {
  const appDirectory = await mkdtemp(path.join(tmpdir(), "zgui-manifest-policy-"));
  t.after(() => rm(appDirectory, { recursive: true, force: true }));
  const src = path.join(appDirectory, "src");
  const packageFile = path.join(appDirectory, "package.json");
  const overlayFile = path.join(appDirectory, "overlay.json");
  await mkdir(src);
  const entry = path.join(src, "main.tsx");
  await writeFile(
    entry,
    `import { renderApp, type NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) { return renderApp(() => <text>Policy</text>, root); }
`,
  );
  await writeFile(
    path.join(src, "worker.ts"),
    "export function run(value: unknown) { return value; }\n",
  );
  const compiled = await bundle(entry, { appDirectory });
  return async (zgui, { extraCapabilities, overlay } = {}) => {
    await writeFile(packageFile, JSON.stringify({ zgui }));
    if (overlay) await writeFile(overlayFile, JSON.stringify(overlay));
    const { manifest } = await publishApplication("policy-fixture", compiled, {
      root: workspaceRoot,
      appDirectory,
      extraCapabilities,
      capabilitiesFile: overlay ? overlayFile : undefined,
    });
    return manifest;
  };
}

test("manifest text and worker declarations agree with package schema boundaries", async t => {
  const publish = await publisher(t);
  for (const { value, valid } of cases.manifestText) {
    checkSchema(value, props.id, valid);
    checkSchema(value, props.window.properties.title, valid);
    if (valid) {
      const manifest = await publish({ id: value, window: { title: value } });
      assert.equal(manifest.id, value);
      assert.equal(manifest.window.title, value);
    } else {
      await assert.rejects(publish({ id: value }), /zgui.id.*trimmed/, JSON.stringify(value));
      await assert.rejects(
        publish({ window: { title: value } }),
        /zgui.window.title.*trimmed/,
        JSON.stringify(value),
      );
    }
  }
  for (const { value, valid } of cases.workerNames) {
    checkSchema(worker(value), props.workers.items, valid);
    const publication = publish({ workers: { [value]: "src/worker.ts" } });
    if (valid) assert.deepEqual((await publication).workers, [worker(value)]);
    else await assert.rejects(publication, /Invalid compute worker name/, JSON.stringify(value));
  }
  for (const { value, valid } of cases.workerCounts) {
    const names = Array.from({ length: value }, (_, i) => `Worker${String(i).padStart(2, "0")}`);
    checkSchema(names.map(worker), props.workers, valid);
    const publication = publish({
      workers: Object.fromEntries(names.map(name => [name, "src/worker.ts"])),
    });
    if (valid) assert.deepEqual((await publication).workers, names.map(worker));
    else await assert.rejects(publication, /maximum of 32 workers/);
  }
});

test("package schema accepts only the versioned QuickJS bytecode format", () => {
  for (const { value, valid } of cases.bytecodeFormats)
    checkSchema(value, props.bytecodeFormat, valid);
});

test("host paths and file grants agree with the canonical package schema", async t => {
  const publish = await publisher(t);
  for (const field of ["imagePrefixes", "unixSockets"]) {
    for (const { value, valid } of cases.hostPaths) {
      checkSchema([value], caps[field], valid);
      const input = { [field]: [value] };
      if (valid) assert.deepEqual((await publish({ capabilities: input })).capabilities, input);
      else await assert.rejects(publish({ capabilities: input }), /contains invalid entries/);
    }
  }
  for (const { value, valid } of cases.directories) {
    const input = { files: { directories: [value] } };
    checkSchema(input, props.capabilities, valid);
    if (valid) assert.deepEqual((await publish({ capabilities: input })).capabilities, input);
    else
      await assert.rejects(
        publish({ capabilities: input }),
        /directories contains invalid entries/,
      );
  }
});

test("filesystem grant ceilings survive sealing and cannot be widened by overlays", async t => {
  const publish = await publisher(t);
  const base = { files: { readOnlyGrants: true, allowBookmarks: false } };
  const overlay = { files: { readOnlyGrants: false, allowBookmarks: true } };
  const { capabilities } = await publish({ capabilities: base }, { overlay });
  assert.deepEqual(capabilities, base);
  checkSchema(capabilities, props.capabilities);
  assert.equal((await publish({ capabilities: overlay })).capabilities, undefined);
  const invalid = { files: { readOnlyGrants: "false" } };
  checkSchema(invalid, props.capabilities, false);
  await assert.rejects(publish({ capabilities: invalid }), /readOnlyGrants must be a boolean/);
});

test("network policy canonicalizes origins and requires independent opt-ins per origin", async t => {
  const publish = await publisher(t);
  for (const { value, expected, error, schemaRejects } of cases.network) {
    if (error) {
      // The schema checks syntax; it cannot classify private hosts or scope a policy opt-in.
      checkSchema(value, caps.network, !schemaRejects);
      await assert.rejects(
        publish({ capabilities: { network: value } }),
        new RegExp(error),
        JSON.stringify(value),
      );
    } else {
      const { capabilities } = await publish({ capabilities: { network: value } });
      assert.deepEqual(capabilities.network, expected);
      checkSchema(capabilities.network, caps.network);
    }
  }
});

test("build-time overlays can grant shell surfaces but cannot grant session lock", async t => {
  const publish = await publisher(t);
  for (const { base, extra, file, expected, error } of cases.overlays) {
    const publication = publish(
      { capabilities: base },
      { extraCapabilities: extra, overlay: file },
    );
    if (error) await assert.rejects(publication, err => err.message.includes(error));
    else {
      const { capabilities } = await publication;
      assert.deepEqual(capabilities, expected);
      checkSchema(capabilities, props.capabilities);
    }
  }
});

test("single-instance launch policy comes from the base package and omits false", async t => {
  const publish = await publisher(t);
  assert.equal(
    (await publish({ capabilities: { singleInstance: false } })).capabilities,
    undefined,
  );
  assert.deepEqual(
    (
      await publish(
        { capabilities: { singleInstance: true } },
        { overlay: { controlChannel: true } },
      )
    ).capabilities,
    { singleInstance: true, controlChannel: true },
  );
  await assert.rejects(
    publish({ capabilities: { singleInstance: "true" } }),
    /singleInstance must be a boolean/,
  );
  for (const overlay of [
    { extraCapabilities: { singleInstance: true } },
    { overlay: { singleInstance: true } },
  ]) {
    await assert.rejects(publish({}, overlay), /singleInstance: true may only be declared in base/);
  }
});
