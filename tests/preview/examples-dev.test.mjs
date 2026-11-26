import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  emptyFixture,
  functionalProfile,
  runScenario,
  writeJson,
} from "./harness.mjs";

let temp;

before(async () => {
  temp = await createTempWorkspace("preview-examples-dev-");
});

after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `dev-step-${++serial}`;
const click = (testId, surface = "main") => ({
  id: id(),
  op: "pointer.click",
  target: { surface, testId },
  button: "primary",
});
const check = (testId, property, equals, surface = "main") => ({
  id: id(),
  op: "assert",
  target: { surface, testId },
  check: { property, equals },
});
const capture = (captureId, surface = "main") => ({
  id: id(),
  op: "capture",
  captureId,
  surface,
});
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

const BASE_CAPS = ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"];

test(
  "md-editor runs in development: true without reactive errors or warnings",
  { timeout: 60000 },
  async () => {
    const bundle = await buildExampleApp("md-editor", { development: true });
    const { result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      profile: functionalProfile,
      fixture: emptyFixture,
      name: "md-editor-dev",
      id: "examples-dev-md-editor",
      width: 960,
      height: 600,
      scale: 1,
      forbidJsDiagnostics: true,
      requiredCapabilities: [...BASE_CAPS, "surface.multiple"],
      steps: [
        advance(40),
        check("md-editor-root", "exists", true),
        check("view-editor", "exists", true),
        click("nav-search"),
        advance(40),
        check("view-search", "exists", true),
        click("nav-settings"),
        advance(40),
        check("view-settings", "exists", true),
        check("settings-font-slider", "exists", true),
        click("btn-about-window"),
        advance(40),
        check("aux-about-root", "exists", true, "about-prefs"),
        click("aux-close-btn", "about-prefs"),
        advance(32),
        click("nav-editor"),
        advance(40),
        check("view-editor", "exists", true),
        capture("md-editor-dev-final"),
      ],
    });
    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "motion-lab runs in development: true without reactive errors or warnings",
  { timeout: 60000 },
  async () => {
    const bundle = await buildExampleApp("motion-lab", { development: true });
    const { result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      profile: functionalProfile,
      fixture: emptyFixture,
      name: "motion-lab-dev",
      id: "examples-dev-motion-lab",
      width: 960,
      height: 600,
      scale: 1,
      forbidJsDiagnostics: true,
      requiredCapabilities: BASE_CAPS,
      steps: [
        advance(40),
        check("motion-lab-root", "exists", true),
        check("motion-arena-stage", "exists", true),
        click("lab-tab-paint"),
        advance(40),
        click("lab-tab-stress"),
        advance(40),
        check("stress-scroll", "exists", true),
        click("lab-tab-arena"),
        advance(40),
        capture("motion-lab-dev-final"),
      ],
    });
    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "workspace runs in development: true without reactive errors or warnings",
  { timeout: 60000 },
  async () => {
    const bundle = await buildExampleApp("workspace", { development: true });
    const { result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      profile: functionalProfile,
      fixture: emptyFixture,
      name: "workspace-dev",
      id: "examples-dev-workspace",
      width: 1040,
      height: 640,
      scale: 1,
      forbidJsDiagnostics: true,
      requiredCapabilities: BASE_CAPS,
      steps: [
        advance(40),
        check("workspace-root", "exists", true),
        click("ws-tab-kanban"),
        advance(40),
        click("ws-tab-assets"),
        advance(40),
        click("ws-tab-notes"),
        advance(40),
        capture("workspace-dev-final"),
      ],
    });
    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "gallery runs in development: true without reactive errors or warnings",
  { timeout: 60000 },
  async () => {
    const bundle = await buildExampleApp("gallery", { development: true });
    const { result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      profile: functionalProfile,
      fixture: emptyFixture,
      name: "gallery-dev",
      id: "examples-dev-gallery",
      width: 1120,
      height: 760,
      scale: 1,
      forbidJsDiagnostics: true,
      requiredCapabilities: BASE_CAPS,
      steps: [
        advance(40),
        check("gallery-page", "exists", true),
        check("section-atoms", "exists", true),
        click("tab-composites"),
        advance(40),
        check("section-composites", "exists", true),
        click("tab-workbench"),
        advance(40),
        check("section-workbench", "exists", true),
        capture("gallery-dev-final"),
      ],
    });
    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "sysmon runs in development: true without reactive errors or warnings",
  { timeout: 60000 },
  async () => {
    const bundle = await buildExampleApp("sysmon", { development: true });
    const fixturePath = path.join(temp.dir, "sysmon-dev-fixture.json");
    await writeJson(fixturePath, {
      schemaVersion: 1,
      id: "sysmon-dev-fixture",
      services: {
        commands: [
          {
            program: "/usr/bin/uname",
            args: ["-srm"],
            mode: "buffered",
            events: [
              { atMs: 2, kind: "output", status: 0, stdout: "Linux 6.12.0 x86_64\n", stderr: "" },
            ],
          },
          {
            program: "/usr/bin/free",
            args: ["-m"],
            mode: "buffered",
            events: [
              {
                atMs: 2,
                kind: "output",
                status: 0,
                stdout:
                  "              total        used        free      shared  buff/cache   available\nMem:          32000        8000       16000         200        8000       23000\nSwap:          8000           0        8000\n",
                stderr: "",
              },
            ],
          },
          {
            program: "/usr/bin/ps",
            args: ["-eo", "pid,pcpu,pmem,rss,stat,comm", "--sort=-pcpu"],
            mode: "buffered",
            events: [
              {
                atMs: 2,
                kind: "output",
                status: 0,
                stdout: "  PID %CPU %MEM   RSS STAT COMMAND\n    1  1.2  0.1 12000 Ss   systemd\n",
                stderr: "",
              },
            ],
          },
          {
            program: "/usr/bin/df",
            args: ["-h", "--output=target,pcent,used,size", "-x", "tmpfs", "-x", "devtmpfs"],
            mode: "buffered",
            events: [
              {
                atMs: 2,
                kind: "output",
                status: 0,
                stdout: "Mounted on Use%  Used  Size\n/           42%  100G  240G\n",
                stderr: "",
              },
            ],
          },
          {
            program: "/usr/bin/vmstat",
            args: ["1"],
            mode: "streaming",
            events: [
              {
                atMs: 10,
                kind: "stdout",
                text: " 1  0      0 16000000 200000 8000000    0    0     0     0 1200 2400  8  3 88  1  0",
              },
            ],
          },
        ],
        control: {},
      },
      events: [],
    });
    const { result } = await runScenario({
      projectDir: temp.dir,
      bundle,
      profile: functionalProfile,
      fixture: fixturePath,
      name: "sysmon-dev",
      id: "examples-dev-sysmon",
      width: 1000,
      height: 640,
      scale: 1,
      forbidJsDiagnostics: true,
      requiredCapabilities: [...BASE_CAPS, "services.fixtures"],
      steps: [
        advance(60),
        check("sysmon-root", "exists", true),
        click("sysmon-tab-processes"),
        advance(40),
        check("sysmon-proc-scroll", "exists", true),
        click("sysmon-tab-storage"),
        advance(40),
        click("sysmon-tab-overview"),
        advance(40),
        capture("sysmon-dev-final"),
      ],
    });
    assert.equal(result.teardown?.clean, true);
  },
);
