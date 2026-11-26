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

  // 1. Parse suites, --test targets, and --lib filters from tools/test-vendor.mjs.
  const registeredCrates = new Set();
  const registeredTests = new Set();
  const registeredLibs = new Map();
  for (const block of runnerSource.matchAll(/\{\s*crate:\s*"([^"]+)",\s*args:\s*\[([^\]]+)\]/gs)) {
    const crate = block[1];
    registeredCrates.add(crate);
    const argsText = block[2];
    const argTokens = [...argsText.matchAll(/"([^"]+)"/g)].map(match => match[1]);
    for (let i = 0; i < argTokens.length; i += 1) {
      if (argTokens[i] === "--test" && i + 1 < argTokens.length) {
        const testName = argTokens[i + 1];
        const filePath = path.join(workspaceRoot, "vendor", crate, "tests", `${testName}.rs`);
        assert.ok(
          existsSync(filePath),
          `tools/test-vendor.mjs references missing test target: vendor/${crate}/tests/${testName}.rs`,
        );
        registeredTests.add(`${crate}:${testName}`);
        i += 1;
      } else if (argTokens[i] === "--lib") {
        let filters = registeredLibs.get(crate);
        if (!filters) {
          filters = new Set();
          registeredLibs.set(crate, filters);
        }
        const next = argTokens[i + 1];
        if (next && !next.startsWith("--")) {
          filters.add(next);
          i += 1;
        } else {
          filters.add("*");
        }
      }
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

  // 4. Every `--lib [filter]` cargo test command in vendor/README.md must be covered by tools/test-vendor.mjs.
  for (const match of vendorReadme.matchAll(
    /cargo\s+test[^\n]*?(?:--manifest-path\s+vendor\/([a-z0-9-]+)\/Cargo\.toml|-p\s+(zgui-[a-z0-9-]+))[^\n]*?--lib(?:\s+([A-Za-z0-9_:]+))?/g,
  )) {
    const crate = match[1] ?? match[2];
    const filter = match[3] ?? "*";
    const covered = registeredLibs.get(crate);
    assert.ok(
      covered &&
        (covered.has("*") ||
          (filter !== "*" && [...covered].some(prefix => filter.startsWith(prefix)))),
      `vendor/README.md documents \`--lib${filter === "*" ? "" : ` ${filter}`}\` for ${crate}, which is not covered in tools/test-vendor.mjs`,
    );
  }
});
