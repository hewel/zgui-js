import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  buildExampleApp,
  createTempWorkspace,
  defaultProfile,
  readMetrics,
  runScenario,
  writeJson,
} from "./harness.mjs";
import { readFile } from "node:fs/promises";
import path from "node:path";

let temp;
let bundle;
let profile;

before(async () => {
  temp = await createTempWorkspace("preview-gallery-fuzz-");
  bundle = await buildExampleApp("gallery");
  const base = JSON.parse(await readFile(defaultProfile, "utf8"));
  profile = path.join(temp.dir, "profile.json");
  await writeJson(profile, {
    ...base,
    limits: { ...base.limits, maxJsEntryWallMs: 30 },
    fonts: { ...base.fonts, manifest: "../../preview/fonts/fonts.json" },
  });
});

after(async () => {
  await temp?.cleanup();
});

function mulberry32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function pick(rng, items) {
  return items[Math.floor(rng() * items.length)];
}

function buildFuzzSteps(seed) {
  const rng = mulberry32(seed);
  let serial = 0;
  const id = () => `fuzz-${seed.toString(16)}-${++serial}`;
  const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
  const key = name => ({ id: id(), op: "key.press", surface: "main", key: name });
  const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
  const wheelY = dy => ({ id: id(), op: "wheel", surface: "main", position: { x: 640, y: 520 }, delta: { x: 0, y: dy, unit: "pixels" } });

  const tabs = ["atoms", "feedback", "fields", "choices", "composites", "overlays", "menus"];
  const presets = ["preset-base", "preset-ocean", "preset-ember", "preset-midnight", "preset-terminal"];

  const steps = [
    { id: id(), op: "surface.resize", surface: "main", width: 1280, height: 960 },
    advance(100),
  ];

  let section = "atoms";
  let openSurface = null;
  let totalAdvanceMs = 100;
  const pushAdvance = ms => {
    totalAdvanceMs += ms;
    steps.push(advance(ms));
  };

  for (let turn = 0; turn < 18 && steps.length < 24 && totalAdvanceMs < 1100; turn++) {
    if (openSurface !== null) {
      if (openSurface === "preset" && rng() < 0.7) {
        steps.push(click(pick(rng, presets)));
        pushAdvance(180);
      } else if (openSurface === "menubar-view" && rng() < 0.5) {
        steps.push(click("menus-editor-spacious"));
        pushAdvance(150);
      } else {
        steps.push(key("Escape"));
        pushAdvance(220);
      }
      openSurface = null;
      continue;
    }

    const roll = rng();
    if (roll < 0.34) {
      const next = pick(rng, tabs);
      section = next;
      steps.push(click(`tab-${next}`));
      pushAdvance(60);
    } else if (roll < 0.48) {
      steps.push(click("scheme-switch"));
      pushAdvance(80);
    } else if (roll < 0.62) {
      steps.push(click("preset-trigger"));
      pushAdvance(160);
      openSurface = "preset";
    } else if (roll < 0.74) {
      const dy = pick(rng, [180, -180, 120, -120]);
      steps.push(wheelY(dy), advance(20), wheelY(-dy));
      totalAdvanceMs += 20;
      pushAdvance(20);
    } else if (section === "atoms") {
      steps.push(click("btn-default"));
      pushAdvance(20);
    } else if (section === "feedback") {
      steps.push(click(rng() < 0.5 ? "progress-plus" : "progress-minus"));
      pushAdvance(20);
    } else if (section === "fields") {
      steps.push(click("inputs-name"), { id: id(), op: "text.commit", surface: "main", text: "Z" });
      pushAdvance(20);
    } else if (section === "choices") {
      steps.push(click("inputs-terms"));
      pushAdvance(20);
    } else if (section === "composites") {
      steps.push(click("avatar-toggle-src"));
      pushAdvance(40);
    } else if (section === "overlays") {
      steps.push(click("overlay-dialog-trigger"));
      pushAdvance(180);
      openSurface = "dialog";
    } else if (section === "menus") {
      if (rng() < 0.5) {
        steps.push(click("menus-dropdown-trigger"));
        pushAdvance(180);
        openSurface = "dropdown";
      } else {
        steps.push(click("menus-editor-view"));
        pushAdvance(150);
        openSurface = "menubar-view";
      }
    }
  }

  if (openSurface !== null) {
    steps.push(key("Escape"), advance(250));
  } else {
    steps.push(key("Escape"), advance(250));
  }
  steps.push({ id: id(), op: "capture", captureId: "settled" });
  return steps;
}

const FUZZ_SEEDS = [0x5eed0001, 0x5eed0002, 0x5eed0003];
const STACK_LIMIT_BYTES = 240 * 1024;
const ENTRY_BUDGET_US = 30_000;

test("gallery survives deterministic seeded interaction fuzzing under tight stack and latency budgets", async () => {
  for (const seed of FUZZ_SEEDS) {
    const name = `seed-${seed.toString(16)}`;
    const steps = buildFuzzSteps(seed);
    const { out } = await runScenario({
      projectDir: temp.dir,
      bundle,
      name,
      id: `gallery-fuzz-${name}`,
      requiredCapabilities: [
        "pixels",
        "virtual-time",
        "selectors.test-id",
        "input.pointer",
        "input.keyboard",
        "input.text",
        "input.wheel",
        "tree.inspect",
        "surface.resize",
        "surface.scale",
      ],
      steps,
      profile,
      maxJsStackBytes: STACK_LIMIT_BYTES,
    });

    const metrics = await readMetrics(out);
    assert.equal(metrics.jsStackLimitBytes, STACK_LIMIT_BYTES);
    assert.ok(metrics.jsHeapBytes > 0, `${name}: expected non-zero jsHeapBytes`);
    assert.ok(
      metrics.maxJsEntryDurationUs > 0 && metrics.maxJsEntryDurationUs < ENTRY_BUDGET_US,
      `${name}: maxJsEntryDurationUs exceeded budget (${metrics.maxJsEntryDurationUs}/${ENTRY_BUDGET_US}us)`,
    );
  }
});
