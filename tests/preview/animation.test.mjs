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
  temp = await createTempWorkspace("preview-animation-");
  bundle = await buildFixtureApp(
    temp.dir,
    "animation",
    new URL("./fixtures/animation.tsx", import.meta.url),
  );
});
after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const capture = (captureId, requiredFrameTimeUs) => ({
  id: id(),
  op: "capture",
  captureId,
  ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }),
});
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `animation-${name}`,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.keyboard",
      "tree.inspect",
    ],
    steps,
    assertEmptyResources: false,
  });
}

test("native animation lifecycle reaches handlers with seconds, pseudo and live state", async () => {
  const { out } = await run(
    [
      capture("initial", 0),
      click("read"),
      check("queried", "text", "false,false,false,false"),
      click("start"),
      click("read"),
      check("queried", "text", "false,true,false,false"),
      capture("running"),
      advance(100),
      check("ends", "text", "fast-paint:0.080:true:1"),
      capture("one-ended"),
      advance(150),
      check("ends", "text", "fast-paint:0.080:true:1|slow-fade:0.200:false:2"),
      click("read"),
      check("queried", "text", "false,false,false,false"),
      capture("both-ended"),
      click("retained"),
      check("retained-value", "text", "slow-fade:0.200::true"),
      click("stop"),
      click("clear"),
      click("start"),
      advance(40),
      click("stop"),
      click("read"),
      check("queried", "text", "false,false,false,false"),
      check("cancels", "text", "fast-paint:false|slow-fade:false"),
      check("ends", "text", ""),
      capture("cancelled"),
      click("transition-start"),
      click("read"),
      check("queried", "text", "false,false,true,false"),
      advance(200),
      check("transition-end", "text", "width:0.160:false"),
      capture("transition-ended"),
      click("transition-stop"),
      click("transition-start"),
      advance(40),
      click("transition-stop"),
      check("transition-cancel", "text", "width:false"),
      click("pseudo-start"),
      click("read"),
      check("queried", "text", "false,false,false,true"),
      advance(180),
      check("pseudo-end", "text", "pseudo-fade:::before:0.120:false"),
      click("read"),
      check("queried", "text", "false,false,false,false"),
      capture("complete"),
    ],
    "lifecycle",
  );
  const bothEnded = await tree(out, "both-ended");
  const transitionEnded = await tree(out, "transition-ended");
  const complete = await tree(out, "complete");
  for (const name of ["sample", "slide", "pseudo"]) {
    const token = get(bothEnded, name).token;
    assert.equal(get(transitionEnded, name).token, token, `${name} remounted by transition-ended`);
    assert.equal(get(complete, name).token, token, `${name} remounted by complete`);
  }
  assert.ok(
    Math.abs(get(transitionEnded, "slide").boundsLogical.width - 240) <= 0.05,
    `slide width at transition-ended: ${get(transitionEnded, "slide").boundsLogical.width}`,
  );
  assert.ok(
    Math.abs(get(complete, "slide").boundsLogical.width - 160) <= 0.05,
    `slide width at complete: ${get(complete, "slide").boundsLogical.width}`,
  );
});
