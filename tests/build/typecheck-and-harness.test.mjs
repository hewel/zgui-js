import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { workspaceRoot } from "../../packages/build/src/build.ts";
import { validateFixtureSpec, validateScenarioSpec } from "../preview/harness.mjs";

const execute = promisify(execFile);

test("typecheck.ts --app checks a single app with CSS Module type sync", async () => {
  const { stdout } = await execute(
    process.execPath,
    [path.join(workspaceRoot, "packages/build/src/typecheck.ts"), "--app", "md-editor"],
    { cwd: workspaceRoot },
  );
  assert.match(stdout, /Typechecked apps\/examples\/md-editor\/tsconfig\.json/);
});

test("preview harness pre-validates scenario and fixture contracts before launching native-host", () => {
  assert.throws(
    () =>
      validateScenarioSpec({
        schemaVersion: 1,
        id: "bad-capture",
        steps: [{ id: "c1", op: "capture", label: "oops" }],
      }),
    /captureId/,
  );
  assert.throws(
    () =>
      validateScenarioSpec({
        schemaVersion: 1,
        id: "bad-clock",
        steps: [{ id: "t1", op: "clock.virtual", byMs: 10 }],
      }),
    /clock\.advance/,
  );
  assert.throws(
    () =>
      validateFixtureSpec({
        schemaVersion: 1,
        services: { sockets: [{ path: "/tmp/x.sock" }] },
      }),
    /events/,
  );
});
