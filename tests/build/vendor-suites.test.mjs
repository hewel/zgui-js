import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { workspaceRoot } from "../../packages/build/src/build.ts";

test("tools/test-vendor.mjs stays in parity with Cargo.toml excludes, vendor/*/tests files, and vendor/README.md", async () => {
  const [cargoToml, runnerSource, vendorReadme] = await Promise.all([
    readFile(path.join(workspaceRoot, "Cargo.toml"), "utf8"),
    readFile(path.join(workspaceRoot, "tools/test-vendor.mjs"), "utf8"),
    readFile(path.join(workspaceRoot, "vendor/README.md"), "utf8"),
  ]);

  // 1. Parse suites and their --test targets from tools/test-vendor.mjs.
  const registeredCrates = new Set();
  const registeredTests = new Set();
  for (const block of runnerSource.matchAll(/\{\s*crate:\s*"([^"]+)",\s*args:\s*\[([^\]]+)\]/gs)) {
    const crate = block[1];
    registeredCrates.add(crate);
    const argsText = block[2];
    for (const target of argsText.matchAll(/"--test",\s*"([^"]+)"/g)) {
      const testName = target[1];
      const filePath = path.join(workspaceRoot, "vendor", crate, "tests", `${testName}.rs`);
      assert.ok(
        existsSync(filePath),
        `tools/test-vendor.mjs references missing test target: vendor/${crate}/tests/${testName}.rs`,
      );
      registeredTests.add(`${crate}:${testName}`);
    }
  }

  // 2. Every excluded vendor/* crate in Cargo.toml must have at least one suite in tools/test-vendor.mjs.
  const excludeMatch = cargoToml.match(/exclude\s*=\s*\[([^\]]+)\]/s);
  assert.ok(excludeMatch, "Cargo.toml must declare [workspace] exclude");
  for (const entry of excludeMatch[1].matchAll(/"vendor\/([^"]+)"/g)) {
    const crate = entry[1];
    // zgui-view has no unit/integration tests registered directly (covered through zgui-view-dom).
    if (crate === "zgui-view") continue;
    assert.ok(
      registeredCrates.has(crate),
      `Vendored crate vendor/${crate} is missing from tools/test-vendor.mjs`,
    );
  }

  // 3. Every vendor/<crate>/tests/<name>.rs file referenced in vendor/README.md must be registered in tools/test-vendor.mjs.
  for (const match of vendorReadme.matchAll(/vendor\/([a-z0-9-]+)\/tests\/([a-z0-9_]+)\.rs/g)) {
    const crate = match[1];
    const testName = match[2];
    if (testName === "mod") continue;
    assert.ok(
      registeredTests.has(`${crate}:${testName}`),
      `vendor/README.md documents vendor/${crate}/tests/${testName}.rs, which must be registered in tools/test-vendor.mjs`,
    );
  }
});
