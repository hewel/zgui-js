import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-hover-"));
  for (const [variant, fixture] of [["main", "hover.tsx"], ["card", "hover-card.tsx"], ["geometry", "hover-geometry.tsx"], ["initial", "hover-initial.tsx"]]) {
    const app = path.join(project, variant);
    await mkdir(path.join(app, "src"), { recursive: true });
    const source = await readFile(fileURLToPath(new URL(`./fixtures/${fixture}`, import.meta.url)), "utf8");
    await writeFile(path.join(app, "src/main.tsx"), source);
    await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
    }));
    await buildApplication(`hover-${variant}`, { appDirectory: app });
  }
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
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const emptyResources = { nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false };

async function run(steps, name, variant = "main") {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `hover-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard",
      "tree.inspect", "surface.scale"],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(project, variant, "dist"),
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
  assert.deepEqual(result.teardown.resources, emptyResources);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};
const contains = (bounds, x, y) => x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height;
const positionerBounds = (t, testId) => {
  const surface = get(t, testId);
  const positioner = t.nodes.find(node => node.token === surface.parent);
  assert.ok(positioner?.boundsLogical, `missing positioner for ${testId}`);
  return positioner.boundsLogical;
};

test("native tooltip intent, handoff and owner removal", async () => {
  const {out} = await run([
    capture("initial",0), click("program"), check("a-button","focused",true), check("a-surface","exists",false),
    key("Tab"), check("b-button","focused",true), check("b-surface","exists",true),
    key("Tab",["Shift"]), check("a-button","focused",true), advance(100), check("a-surface","role","tooltip"), check("b-surface","exists",false), capture("keyboard"),
    key("Escape"), advance(100), check("a-surface","exists",false), check("a-button","focused",true),
    click("outside"), move(150,218), advance(199), check("a-surface","exists",false), advance(1), check("a-surface","exists",true), advance(100), capture("hover-light"),
    key("End"), check("a-value","text","Alpha 1"), key("Home"), advance(250), check("a-value","text","Alpha 1"), capture("hover-dark"),
    move(330,218), check("b-surface","exists",true), capture("handoff-start"), advance(1), check("a-surface","exists",false), advance(100), capture("handoff"),
    move(20,480), advance(119), check("b-state","text","open"), advance(1), check("b-state","text","closed"), advance(30), check("b-surface","exists",true), capture("exit"), advance(100), check("b-surface","exists",false),
    move(150,218), advance(300), check("a-surface","exists",true), capture("before-retire"),
    key("Delete"), check("a-trigger","exists",false), check("a-surface","exists",false), move(20,480), advance(100),
    move(330,218), advance(199), check("b-surface","exists",false), advance(1), check("b-surface","exists",true),
    key("Delete"), check("a-trigger","exists",true), advance(100), capture("restored")
  ],"intent","main");
  const frames = {};
  for (const name of ["keyboard","hover-light","hover-dark","handoff","exit","before-retire","restored"]) frames[name] = await tree(out, name);
  assert.equal(get(frames["hover-light"], "a-surface").token, get(frames["hover-dark"], "a-surface").token, "theme swap must not remount the surface");
  assert.equal(get(frames["hover-light"], "a-value").token, get(frames["hover-dark"], "a-value").token);
  assert.equal(get(frames["hover-dark"], "a-value").text, "Alpha 1");
  assert.notEqual(get(frames["keyboard"], "a-surface").token, get(frames["hover-light"], "a-surface").token, "a completed exit remounts");
  assert.notEqual(get(frames["before-retire"], "a-trigger").token, get(frames["restored"], "a-trigger").token, "restored source is a fresh trigger");
  assert.equal(get(frames["handoff"], "b-surface").token, get(frames["exit"], "b-surface").token, "b surface persists through the leaving frame");
  const anchor = get(frames["hover-light"], "a-trigger").boundsLogical;
  const surface = get(frames["hover-light"], "a-surface").boundsLogical;
  const expectedOffset = 10 * Math.SQRT2 / 2 + 3;
  assert.ok(Math.abs(anchor.y - surface.y - surface.height - expectedOffset) <= 0.65,
    `arrow-clamped offset gap: ${JSON.stringify({ anchor, surface })}`);
  assert.equal(get(frames["hover-light"], "outside").state.focused, true, "hover must not steal focus");
  assert.equal(get(frames["hover-dark"], "outside").state.focused, true, "theme swap keeps focus");
});

test("native hover card delays, stays open across the surface and never traps focus", async () => {
  const {out} = await run([
    capture("initial",0), click("before"), move(180,378), advance(699), check("card","exists",false), advance(1), check("card","exists",true), check("before","focused",true), advance(200), capture("opened"),
    move(180,420), advance(200), check("card","exists",true), key("Escape"), advance(1000), check("card","exists",false), check("before","focused",true),
    move(180,378), advance(900), check("card-value","text","Preview 2"), capture("fresh"), move(180,420), advance(200),
    click("before"), check("state","text","open"), advance(299), check("state","text","open"), advance(1), check("state","text","closed"), advance(40), check("card","exists",true), capture("exit"),
    key("Tab"), check("card-trigger","focused",true), check("state","text","open"), check("card-value","text","Preview 2"), capture("reopened"),
    advance(200), check("card-value","text","Preview 2"), capture("reopened-settled"),
    key("Tab"), check("after","focused",true), check("state","text","closed"), advance(250), check("card","exists",false), capture("closed")
  ],"card","card");
  const frames = {};
  for (const name of ["opened","fresh","exit","reopened","reopened-settled","closed"]) frames[name] = await tree(out, name);
  assert.notEqual(get(frames["opened"], "card").token, get(frames["fresh"], "card").token, "a completed exit remounts");
  for (const [a, b] of [["fresh", "exit"], ["fresh", "reopened"], ["fresh", "reopened-settled"]])
    assert.equal(get(frames[a], "card").token, get(frames[b], "card").token, `${a} vs ${b}: card identity`);
  assert.equal(get(frames["fresh"], "card-value").token, get(frames["reopened"], "card-value").token);
  assert.equal(get(frames["fresh"], "card-value").token, get(frames["reopened-settled"], "card-value").token);
  assert.equal(get(frames["opened"], "card").role, "dialog");
  assert.ok(contains(get(frames["opened"], "card").boundsLogical, 180, 420), "hover point must land inside the card");
  assert.ok(contains(get(frames["opened"], "card-trigger").boundsLogical, 180, 378), "hover point must land inside the anchor");
});

test("native tooltip arrow tracks anchors on every side through theme and density changes", async () => {
  const {out} = await run([capture("initial",0), advance(300), capture("light"), click("theme"), advance(300), capture("dark"),
    {id:id(),op:"surface.scale",surface:"main",scale:2}, advance(50), capture("dense")],"geometry","geometry");
  const frames = {};
  for (const name of ["initial","light","dark","dense"]) frames[name] = await tree(out, name);
  const offset = 10 * Math.SQRT2 / 2 + 3;
  const edgeGap = (side, a, s) => side === "top" ? a.y - s.y - s.height : side === "bottom" ? s.y - a.y - a.height
    : side === "left" ? a.x - s.x - s.width : s.x - a.x - a.width;
  const actualSide = (a, s) => s.y + s.height <= a.y + 0.5 ? "top" : s.y >= a.y + a.height - 0.5 ? "bottom"
    : s.x + s.width <= a.x + 0.5 ? "left" : "right";
  const hints = [["flipped", "top", "bottom"], ["top", "top", "top"], ["left", "left", "left"], ["right", "right", "right"]];
  assert.equal(frames.initial.nodes.some(node => node.state?.focused), false, "hover surfaces install no focus trap");
  for (const [hint, , expected] of hints) {
    const tokens = new Map();
    for (const name of ["initial","light","dark","dense"]) {
      const t = frames[name];
      const anchor = get(t, `${hint}-anchor`).boundsLogical;
      const surface = name === "initial" ? positionerBounds(t, `${hint}-surface`) : get(t, `${hint}-surface`).boundsLogical;
      assert.equal(get(t, `${hint}-surface`).role, "tooltip", `${name} ${hint}: role`);
      const side = actualSide(anchor, surface);
      assert.equal(side, expected, `${name} ${hint}: requested side must resolve to ${expected}`);
      assert.ok(Math.abs(edgeGap(side, anchor, surface) - offset) <= 0.7,
        `${name} ${hint}: panel gap ${edgeGap(side, anchor, surface)} vs ${offset}`);
      if (name === "initial") {
        assert.deepEqual(surface, get(frames.light, `${hint}-surface`).boundsLogical,
          `${hint}: first-paint positioner already matches the settled surface`);
      } else {
        const arrow = get(t, `${hint}-arrow`).boundsLogical;
        assert.ok(Math.abs(edgeGap(side, anchor, arrow) - 4) <= 1.1,
          `${name} ${hint}: arrow tip gap ${edgeGap(side, anchor, arrow)} vs 4`);
        if (side === "top" || side === "bottom") {
          const edge = side === "top" ? surface.y + surface.height : surface.y;
          assert.ok(arrow.y < edge && arrow.y + arrow.height > edge,
            `${name} ${hint}: arrow must straddle the ${side} panel edge ${JSON.stringify({ arrow, edge })}`);
          const clamped = Math.min(Math.max(anchor.x + anchor.width / 2, surface.x + 10), surface.x + surface.width - 10);
          assert.ok(Math.abs(arrow.x + arrow.width / 2 - clamped) <= 1.2,
            `${name} ${hint}: arrow center ${arrow.x + arrow.width / 2} vs clamped anchor ${clamped}`);
        } else {
          const edge = side === "left" ? surface.x + surface.width : surface.x;
          assert.ok(arrow.x < edge && arrow.x + arrow.width > edge,
            `${name} ${hint}: arrow must straddle the ${side} panel edge ${JSON.stringify({ arrow, edge })}`);
          const clamped = Math.min(Math.max(anchor.y + anchor.height / 2, surface.y + 10), surface.y + surface.height - 10);
          assert.ok(Math.abs(arrow.y + arrow.height / 2 - clamped) <= 1.2,
            `${name} ${hint}: arrow center ${arrow.y + arrow.height / 2} vs clamped anchor ${clamped}`);
        }
      }
      for (const part of ["surface", "arrow"])
        tokens.set(part, (tokens.get(part) ?? []).concat(get(t, `${hint}-${part}`).token));
    }
    assert.equal(new Set(tokens.get("surface")).size, 1, `${hint}: surface remounted across theme/density`);
    assert.equal(new Set(tokens.get("arrow")).size, 1, `${hint}: arrow remounted across theme/density`);
  }
  for (const name of ["initial","light","dark","dense"]) {
    const flipped = name === "initial" ? positionerBounds(frames[name], "flipped-surface") : get(frames[name], "flipped-surface").boundsLogical;
    const anchor = get(frames[name], "flipped-anchor").boundsLogical;
    assert.ok(flipped.x >= 7.4 && flipped.y >= 7.4 && flipped.x + flipped.width <= 800 - 7.4 && flipped.y + flipped.height <= 600 - 7.4,
      `${name}: flipped surface leaves the viewport margin ${JSON.stringify(flipped)}`);
    assert.ok(Math.abs(flipped.x + flipped.width / 2 - (anchor.x + anchor.width / 2)) > 1,
      `${name}: flipped panel must shift away from the anchor center`);
  }
});

test("default-open hover surfaces inside a dialog escape and close in order", async () => {
  await run([capture("initial",0), advance(300), check("tip-button","focused",true), check("tip","role","tooltip"), check("card","role","dialog"), capture("open"),
    key("Escape"), advance(200), check("card","exists",false), check("tip","exists",true), check("tip-button","focused",true),
    key("Escape"), advance(100), check("tip","exists",false), check("dialog","exists",true),
    key("Tab"), check("card-trigger","focused",true), check("card","exists",true),
    key("Tab"), check("close","focused",true), advance(200), check("card","exists",false),
    key("Escape"), advance(300), check("dialog","exists",false), capture("closed")],"initial","initial");
});
