import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { validateModule } from "../../packages/build/src/validate.ts";
import { createHost, moduleFor } from "./native-fixture.mjs";

test("@hewel/zgui/shell-host exports privileged lock, entry loading, handoff, and status hooks", async t => {
  const pkgJson = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/ui/package.json"), "utf8"),
  );
  assert.deepEqual(pkgJson.exports["./shell-host"], {
    types: "./dist/shell-host.d.ts",
    default: "./dist/shell-host.js",
  });

  const source = `
import {
  SessionLockSurface,
  getOverlayStatus,
  listApprovedExtensionEntries,
  listApprovedPluginEntries,
  loadApprovedExtensionEntry,
  loadApprovedPluginEntry,
  onBeforeSessionCommit,
  onSessionCommitted,
  registerSessionHandoff,
  registerSharedRuntimeModules,
  requestSessionLock,
  takeSessionHandoff,
} from "@hewel/zgui/shell-host";

export function verifyExports() {
  return {
    sameLoad: loadApprovedPluginEntry === loadApprovedExtensionEntry,
    sameList: listApprovedPluginEntries === listApprovedExtensionEntries,
    hasLockSurface: typeof SessionLockSurface === "function",
    hasRequestLock: typeof requestSessionLock === "function",
    hasHandoff: typeof registerSessionHandoff === "function" && typeof takeSessionHandoff === "function",
    hasLifecycle: typeof onSessionCommitted === "function" && typeof onBeforeSessionCommit === "function",
    hasStatus: typeof getOverlayStatus === "function",
    hasSharedReg: typeof registerSharedRuntimeModules === "function",
    entries: listApprovedPluginEntries(),
    handoff: takeSessionHandoff(),
    status: getOverlayStatus(),
  };
}
`;
  const mod = await moduleFor(t, source);
  const previousShellHost = globalThis.__zguiShellHost;
  globalThis.__zguiShellHost = {
    listApprovedEntries: () => [
      {
        entryId: "acme/clock:widget",
        extensionId: "acme/clock",
        version: "1.0.0",
        kind: "panel",
        bundle: "dist/entries/widget.mjs",
        canonicalSpecifier: "zgui-ext://acme/clock/dist/entries/widget.mjs",
      },
    ],
    loadApprovedEntry: async entryId => ({ entryId, loaded: true }),
    registerSharedRuntimeModules: () => {},
    registerSessionHandoff: () => () => {},
    takeSessionHandoff: () => ({ restored: 1 }),
    onSessionCommitted: () => () => {},
    onBeforeCommit: () => () => {},
    getOverlayStatus: () => ({
      generation: 3,
      overlayId: "ovl-1",
      phase: "committed",
      safeBaseShell: false,
      activeExtensionIds: ["acme/clock"],
      clientMetadata: { tag: "opaque" },
      lastFailure: null,
    }),
  };
  t.after(() => {
    if (previousShellHost === undefined) delete globalThis.__zguiShellHost;
    else globalThis.__zguiShellHost = previousShellHost;
  });

  const summary = mod.verifyExports();
  assert.equal(summary.sameLoad, true);
  assert.equal(summary.sameList, true);
  assert.equal(summary.hasLockSurface, true);
  assert.equal(summary.hasRequestLock, true);
  assert.equal(summary.hasHandoff, true);
  assert.equal(summary.hasLifecycle, true);
  assert.equal(summary.hasStatus, true);
  assert.equal(summary.hasSharedReg, true);
  assert.equal(summary.entries[0].entryId, "acme/clock:widget");
  assert.deepEqual(summary.handoff, { restored: 1 });
  assert.equal(summary.status.overlayId, "ovl-1");

  const harness = createHost();
  assert.ok(harness);
});

test("ordinary dynamic import() strict static-literal and closed-chunk-graph validation remains enforced", async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  const dir = await mkdtemp(path.join(workspaceRoot, "target", "zgui-validate-inv-"));
  try {
    await mkdir(path.join(dir, "src"), { recursive: true });
    await writeFile(
      path.join(dir, "tsconfig.json"),
      JSON.stringify(
        { extends: "../../packages/ui/tsconfig.native.json", include: ["src"] },
        null,
        2,
      ),
    );
    await writeFile(
      path.join(dir, "package.json"),
      JSON.stringify({
        name: "validate-inv",
        private: true,
        zgui: { id: "dev.test.validate-inv" },
      }),
    );

    // 1. Variable dynamic import() must still fail validation.
    await writeFile(
      path.join(dir, "src/main.tsx"),
      `import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const target = "./lazy.mjs";
  void import(target);
  return renderApp(() => <text>ok</text>, root);
}
`,
    );
    await assert.rejects(
      () => buildApplication("validate-inv", { root: workspaceRoot, appDirectory: dir }),
      /Dynamic imports must use a static string literal specifier/,
    );

    // 2. Outside-graph static dynamic import() must still fail closed-graph validation.
    assert.throws(
      () =>
        validateModule(
          `export const hostApiVersion = 1;\nexport function mount() { void import("./missing-outside-chunk.mjs"); }\n`,
          {
            filename: "app.mjs",
            bundled: true,
            entry: false,
            allowedChunks: new Set(),
          },
        ),
      /Bundle contains an unresolved dynamic import/,
    );

    // 3. Importing from @hewel/zgui/shell-host inside a base shell app bundles and typechecks cleanly.
    await writeFile(
      path.join(dir, "src/main.tsx"),
      `import { renderShell } from "@hewel/zgui";
import { getOverlayStatus, listApprovedPluginEntries } from "@hewel/zgui/shell-host";
export const hostApiVersion = 1;
export function mount() {
  void listApprovedPluginEntries();
  void getOverlayStatus();
  return renderShell(() => null);
}
`,
    );
    const published = await buildApplication("validate-inv", {
      root: workspaceRoot,
      appDirectory: dir,
    });
    assert.equal(published.manifest.id, "dev.test.validate-inv");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
