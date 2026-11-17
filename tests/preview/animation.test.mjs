import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-animation-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/animation.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("animation", { appDirectory: project });
});
after(async () => {
  if (!project) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview project: ${project}`);
  } else {
    await rm(project, { recursive: true, force: true });
  }
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `animation-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect"],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(project, "dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  if (process.stderr) await writeFile(path.join(project, `${name}.stderr`), process.stderr);
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null, process.stderr);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.assertions.passed, steps.filter(step => step.op === "assert").length);
  assert.equal(result.teardown.clean, true);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("native animation lifecycle reaches handlers with seconds, pseudo and live state", async () => {
  const { out } = await run([
    capture("initial", 0), click("read"), check("queried", "text", "false,false,false,false"),
    click("start"), click("read"), check("queried", "text", "false,true,false,false"), capture("running"),
    advance(100), check("ends", "text", "fast-paint:0.080:true:1"), capture("one-ended"),
    advance(150), check("ends", "text", "fast-paint:0.080:true:1|slow-fade:0.200:false:2"),
    click("read"), check("queried", "text", "false,false,false,false"), capture("both-ended"),
    click("retained"), check("retained-value", "text", "slow-fade:0.200::true"),
    click("stop"), click("clear"), click("start"), advance(40), click("stop"),
    click("read"), check("queried", "text", "false,false,false,false"),
    check("cancels", "text", "fast-paint:false|slow-fade:false"), check("ends", "text", ""), capture("cancelled"),
    click("transition-start"), click("read"), check("queried", "text", "false,false,true,false"),
    advance(200), check("transition-end", "text", "width:0.160:false"), capture("transition-ended"),
    click("transition-stop"), click("transition-start"), advance(40), click("transition-stop"),
    check("transition-cancel", "text", "width:false"),
    click("pseudo-start"), click("read"), check("queried", "text", "false,false,false,true"),
    advance(180), check("pseudo-end", "text", "pseudo-fade:::before:0.120:false"),
    click("read"), check("queried", "text", "false,false,false,false"), capture("complete"),
  ], "lifecycle");
  const bothEnded = await tree(out, "both-ended");
  const transitionEnded = await tree(out, "transition-ended");
  const complete = await tree(out, "complete");
  for (const name of ["sample", "slide", "pseudo"]) {
    const token = get(bothEnded, name).token;
    assert.equal(get(transitionEnded, name).token, token, `${name} remounted by transition-ended`);
    assert.equal(get(complete, name).token, token, `${name} remounted by complete`);
  }
  assert.ok(Math.abs(get(transitionEnded, "slide").boundsLogical.width - 240) <= 0.05,
    `slide width at transition-ended: ${get(transitionEnded, "slide").boundsLogical.width}`);
  assert.ok(Math.abs(get(complete, "slide").boundsLogical.width - 160) <= 0.05,
    `slide width at complete: ${get(complete, "slide").boundsLogical.width}`);
});
