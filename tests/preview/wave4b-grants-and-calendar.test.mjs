import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  functionalProfile,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
  workspaceRoot,
  writeJson,
} from "./harness.mjs";

const grantsApp = `
import { renderApp } from "@hewel/zgui";
import { Button } from "@hewel/zgui/components";
import type { NativeNode } from "@hewel/zgui";
import { createSignal } from "solid-js";
import { clearGrants, listGrants, listBookmarks, openAuthorizedDirectory, persistHandle, restoreBookmark, revokeGrant, revokeBookmark } from "@hewel/zgui-services";

export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const [status, setStatus] = createSignal("Ready");
  async function run() {
    const dir = openAuthorizedDirectory("/work");
    const token = await persistHandle(dir).promise.finally(() => dir.close());
    const grants = await listGrants().promise;
    const legacy = await listBookmarks().promise;
    const restore = async (token: string) => {
      try {
        const handle = await restoreBookmark(token).promise;
        const kind = handle.kind;
        handle.close();
        return kind;
      } catch (e) { return (e as {code: string}).code; }
    };
    const workspace = await restore(token);
    const write = await restore("old-write");
    const system = await restore("system");
    const legacySystem = await revokeBookmark("system").promise;
    const removed = await revokeGrant(token).promise;
    const revoked = await restore(token);
    const remaining = await listGrants().promise;
    const cleared = await clearGrants().promise;
    const empty = await listGrants().promise;
    setStatus([grants.map(g => g.kind).join(","), legacy.map(g => g.kind).join(","), workspace, write, system, legacySystem, removed, revoked, remaining.map(g => g.token).join(","), cleared, empty.length].join("|"));
  }
  return renderApp(() => <column><Button testId="run" onClick={() => void run().catch(e => setStatus(String(e)))}>Run</Button><text testId="status">{status()}</text></column>, root);
}
`;

test("fixture grants share the legacy bookmark ledger without bypassing the sealed workspace ceiling", async t => {
  const temp = await createTempWorkspace("preview-wave4b-grants-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "wave4b-grants", grantsApp, {
    rawSource: true,
    capabilities: {
      files: { directories: [{ path: "/work", mode: "read" }], readOnlyGrants: true },
    },
  });
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "wave4b-grants",
    events: [],
    services: {
      files: {
        dirs: ["/work", "/old", "/system"],
        bookmarks: [{ token: "old-write", kind: "dir", path: "/old", mode: "readwrite" }],
        grants: [
          {
            token: "system",
            kind: "system-dir",
            path: "/system",
            mode: "read",
            label: "System",
            createdMs: 42,
          },
        ],
      },
    },
  });
  const service = (method, count, errorCode) => ({
    id: method.toLowerCase(),
    op: "assert",
    serviceCall: { service: "files", method, count, errorCode },
  });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "grants",
    profile: functionalProfile,
    fixture,
    requiredCapabilities: [],
    steps: [
      {
        id: "go",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "run" },
      },
      { id: "settled", op: "clock.advance", byMs: 32 },
      service("listGrants", 3),
      service("clearGrants", 1),
      service("revokeGrant", 1),
      service("restoreBookmark", 4, "E_CAPABILITY"),
      { id: "done", op: "capture", captureId: "done", surface: "main" },
    ],
  });
  assert.equal(
    getNode(await readTree(out, "done"), "status").text,
    "workspace-dir,system-dir,workspace-dir|dir,dir|dir|E_CAPABILITY|E_CAPABILITY|false|true|NotFound|old-write,system|2|0",
  );
});

const calendarApp = `
import { createSignal, renderApp, Show } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Calendar, DatePicker, ThemeProvider } from "@hewel/zgui/components";
import type { DateRangeValue } from "@hewel/zgui/components";

export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const [view, setView] = createSignal<"single" | "locales" | "range" | "picker">("single");
  const [date, setDate] = createSignal<string | null>(null);
  const [range, setRange] = createSignal<DateRangeValue | null>(null);
  const [month, setMonth] = createSignal("2024-02");
  return renderApp(() => <ThemeProvider><column style="padding: 12px">
    <row>
      <Button testId="locales" onClick={() => setView("locales")}>Locales</Button>
      <Button testId="range" onClick={() => setView("range")}>Range</Button>
      <Button testId="picker" onClick={() => setView("picker")}>Picker</Button>
    </row>
    <Show when={view() === "single"}>
      <Calendar locale="en-US" month={month()} onMonthChange={setMonth} value={date()} onValueChange={setDate}/>
      <text testId="chosen">{date() ?? "none"}</text>
    </Show>
    <Show when={view() === "locales"}>
      <row>
        <Calendar testId="german" locale="de-DE" defaultMonth="2026-02"/>
        <Calendar testId="chinese" locale="zh-CN" defaultMonth="2026-02"/>
      </row>
    </Show>
    <Show when={view() === "range"}>
      <Calendar mode="range" numberOfMonths={2} defaultMonth="2024-02" value={range()} onValueChange={setRange}/>
      <text testId="range-chosen">{range() ? range()!.start + "/" + (range()!.end ?? "pending") : "none"}</text>
    </Show>
    <Show when={view() === "picker"}>
      <DatePicker triggerTestId="launch" contentTestId="surface" calendarTestId="picker-calendar"
        locale="en-US" defaultValue="2024-02-28" placeholder="Choose date"
        captionLayout="dropdown" yearRange={[2022, 2022]} clearable/>
    </Show>
  </column></ThemeProvider>, root);
}
`;

test("native calendar localizes weeks, paints virtual today and hover ranges, and preserves keyboard focus", async t => {
  const temp = await createTempWorkspace("preview-wave4b-calendar-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "wave4b-calendar", calendarApp, {
    rawSource: true,
  });
  const profile = path.join(temp.dir, "profile.json");
  const config = JSON.parse(await readFile(functionalProfile, "utf8"));
  config.surface.scale = 1;
  config.limits.maxOutputBytes = 128 * 1024 * 1024;
  config.clock.epochMs = Date.UTC(2024, 2, 1, 0, 30);
  config.fonts.manifest = path.relative(
    temp.dir,
    path.join(workspaceRoot, "preview/fonts/fonts.json"),
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(profile, config);
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "calendar",
    services: {},
    events: [],
    timeZone: "America/Los_Angeles",
  });

  let n = 0;
  const main = "main";
  const step = (op, params) => ({ id: `step-${++n}`, op, ...params });
  const testId = id => ({ surface: main, testId: id });
  const day = name => ({ surface: main, role: "button", name });
  const click = target => step("pointer.click", { target, button: "primary" });
  const move = target => step("pointer.move", { target });
  const key = (key, modifiers = []) => step("key.press", { surface: main, key, modifiers });
  const check = (target, property, equals) =>
    step("assert", { target, check: { property, equals } });
  const focused = date => check(day(date), "focused", true);
  const capture = captureId => step("capture", { captureId, surface: main });
  const advance = byMs => step("clock.advance", { byMs });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "calendar",
    profile,
    fixture,
    requiredCapabilities: ["pixels"],
    forbidJsDiagnostics: true,
    steps: [
      capture("today"),
      click(day("2024-02-29")),
      check(testId("chosen"), "text", "2024-02-29"),
      key("ArrowRight"),
      focused("2024-03-01"),
      key("ArrowDown"),
      focused("2024-03-08"),
      key("ArrowUp"),
      focused("2024-03-01"),
      key("ArrowLeft"),
      focused("2024-02-29"),
      key("Home"),
      focused("2024-02-25"),
      key("End"),
      focused("2024-03-02"),
      key("PageUp"),
      focused("2024-02-02"),
      key("PageDown"),
      focused("2024-03-02"),
      key("PageUp", ["Shift"]),
      focused("2023-03-02"),
      key("PageDown", ["Shift"]),
      focused("2024-03-02"),
      key("Space"),
      check(testId("chosen"), "text", "2024-03-02"),
      click(testId("locales")),
      capture("locales"),
      click(testId("range")),
      click(day("2024-02-29")),
      check(testId("range-chosen"), "text", "2024-02-29/pending"),
      capture("before-hover"),
      move(day("2024-03-02")),
      advance(250),
      capture("hover"),
      check(testId("range-chosen"), "text", "2024-02-29/pending"),
      click(day("2024-03-02")),
      check(testId("range-chosen"), "text", "2024-02-29/2024-03-02"),
      move(testId("range")),
      advance(250),
      capture("committed"),
      click(testId("picker")),
      click(testId("launch")),
      advance(250),
      check(testId("picker-calendar"), "exists", true),
      focused("2024-02-28"),
      capture("picker-open"),
      click(day("2024-02-29")),
      advance(250),
      check(testId("surface"), "exists", false),
      check(testId("launch"), "focused", true),
      check(testId("launch"), "text", "Feb 29, 2024"),
      click(testId("launch")),
      advance(250),
      key("Escape"),
      advance(250),
      check(testId("surface"), "exists", false),
      check(testId("launch"), "focused", true),
      check(testId("launch"), "text", "Feb 29, 2024"),
      click(testId("launch")),
      advance(250),
      step("pointer.click", { surface: main, position: { x: 750, y: 550 }, button: "primary" }),
      advance(250),
      check(testId("surface"), "exists", false),
      check(testId("launch"), "focused", true),
      click(day("Clear date")),
      check(testId("launch"), "text", "Choose date"),
      click(testId("launch")),
      advance(250),
      click(day("Next month")),
      check({ surface: main, role: "grid", name: "March 2024" }, "exists", true),
      click({ surface: main, role: "comboBox", name: "Month" }),
      key("ArrowDown"),
      key("Enter"),
      check({ surface: main, role: "grid", name: "April 2024" }, "exists", true),
      click({ surface: main, role: "comboBox", name: "Year" }),
      check({ surface: main, role: "listBoxOption", name: "2023" }, "exists", false),
      key("Enter"),
      check({ surface: main, role: "grid", name: "April 2022" }, "exists", true),
      key("Escape"),
      advance(250),
      check(testId("launch"), "focused", true),
    ],
  });

  const today = await readTree(out, "today");
  const find = (tree, name) =>
    tree.nodes.find(node => node.role === "button" && node.name === name);
  const headers = (tree, scope) => {
    const descendants = new Set([scope?.token]);
    return tree.nodes
      .filter(node => {
        if (scope && !descendants.has(node.parent)) return false;
        descendants.add(node.token);
        return node.role === "columnHeader";
      })
      .sort((a, b) => a.boundsLogical.x - b.boundsLogical.x)
      .map(node => node.text);
  };
  assert.equal(headers(today)[0], "Sun");
  const locales = await readTree(out, "locales");
  assert.match(headers(locales, getNode(locales, "german"))[0], /^Mo/);
  assert.match(headers(locales, getNode(locales, "chinese"))[0], /一/);
  assert.ok(locales.nodes.some(node => node.text === "Februar 2026"));
  assert.ok(locales.nodes.some(node => node.text === "二月 2026" || node.text === "2月 2026"));
  assert.ok(find(locales, "2026-02-28"));
  assert.equal(find(locales, "2026-02-29"), undefined);

  const fill = async (captureId, name, tree = null) => {
    const { boundsDevice: rect } = find(tree ?? (await readTree(out, captureId)), name);
    const png = await readPng(path.join(out, "captures", `${captureId}.png`));
    return pixelAt(
      png,
      Math.floor(rect.x + rect.width * 0.2),
      Math.floor(rect.y + rect.height / 2),
    );
  };
  assert.notDeepEqual(
    await fill("today", "2024-02-29", today),
    await fill("today", "2024-02-28", today),
  );
  const hover = await fill("hover", "2024-03-01");
  assert.notDeepEqual(hover, await fill("before-hover", "2024-03-01"));
  assert.deepEqual(hover, await fill("committed", "2024-03-01"));
});
