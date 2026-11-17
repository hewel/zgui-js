import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode as get,
  readTree as tree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-presence-");
  bundle = await buildFixtureApp(temp.dir, "presence", new URL("./fixtures/presence.tsx", import.meta.url));
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `presence-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect"],
    steps,
    assertEmptyResources: false,
  });
}

test("presence keeps a surface through its exit animation and releases it on its last motion", async () => {
  const { out } = await run([
    capture("initial", 0), check("surface", "exists", false),
    click("open"), advance(100), click("counter"), check("counter", "text", "1"), capture("opened"),
    click("close"), check("surface", "exists", true), check("state", "text", "closed"), capture("leaving"),
    advance(80), check("surface", "exists", true), capture("mid-exit"),
    click("open"), check("state", "text", "open"), check("counter", "text", "1"),
    advance(200), check("surface", "exists", true), capture("reopened"),
    click("close"), advance(220), check("surface", "exists", false), capture("closed"),
    click("open"), advance(100), check("counter", "text", "0"), capture("fresh"),
    click("close"), advance(40), click("none"), check("surface", "exists", false), capture("cancelled"),
    click("open"), click("close"), check("surface", "exists", false),
    click("zero"), click("open"), click("close"), check("surface", "exists", false), capture("zero"),
    click("forever"), click("open"), click("close"), advance(900), check("surface", "exists", true),
    advance(120), check("surface", "exists", false), capture("deadline"),
    click("animated"), click("open"), advance(100), click("close"), click("source-toggle"),
    check("surface", "exists", false), advance(1200), check("surface", "exists", false), capture("removed"),
    click("source-toggle"), check("surface", "exists", false), click("open"), advance(100),
    check("surface", "exists", true), check("counter", "text", "0"), capture("remounted"),
  ], "lifecycle");
  const opened = await tree(out, "opened");
  const leaving = await tree(out, "leaving");
  const midExit = await tree(out, "mid-exit");
  const reopened = await tree(out, "reopened");
  const fresh = await tree(out, "fresh");
  const remounted = await tree(out, "remounted");
  for (const name of ["surface", "counter"]) {
    const token = get(opened, name).token;
    assert.equal(get(leaving, name).token, token, `${name} remounted while leaving`);
    assert.equal(get(midExit, name).token, token, `${name} remounted mid-exit`);
    assert.equal(get(reopened, name).token, token, `${name} remounted on reopen`);
    assert.notEqual(get(fresh, name).token, token, `${name} not rebuilt after full removal`);
    assert.notEqual(get(remounted, name).token, get(fresh, name).token, `${name} not rebuilt on remount`);
  }
});

test("a presence leaving with active timers tears its session down cleanly", async () => {
  await run([click("open"), advance(100), click("close"), check("surface", "exists", true),
    capture("active-exit")], "active-exit");
});
