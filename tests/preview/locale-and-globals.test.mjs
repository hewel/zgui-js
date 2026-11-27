import assert from "node:assert/strict";
import { test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  functionalProfile,
  runScenario,
  writeJson,
} from "./harness.mjs";

const formattingApp = `
import { renderApp, type NativeNode } from "@hewel/zgui";
import { formatDateTime, getCalendarLabels } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const epoch = Date.UTC(2026, 0, 5, 19, 24, 7);
  const timeZone = "UTC";
  const en = formatDateTime(epoch, { locale: "en-US", timeZone, dateStyle: "full", timeStyle: "short", hourCycle: "h12" });
  const zh = formatDateTime(epoch, { locale: "zh-CN", timeZone, dateStyle: "full" });
  const de = formatDateTime(epoch, { locale: "de-DE", timeZone, dateStyle: "full", timeStyle: "long", hourCycle: "h23" });
  if (!en.includes("Monday") || !en.includes("January") || !en.includes("7:24") || !en.includes("PM")) throw Error(en);
  if (!zh.includes("2026年1月5日") || !zh.includes("星期一")) throw Error(zh);
  if (!de.includes("Montag") || !de.includes("Januar") || !de.includes("19:24:07") || !de.includes("UTC")) throw Error(de);
  const enLabels = getCalendarLabels({ locale: "en-US" });
  const zhLabels = getCalendarLabels({ locale: "zh-CN" });
  const deLabels = getCalendarLabels({ locale: "DE-de" });
  const narrow = getCalendarLabels({ locale: "en-US", width: "narrow" });
  const short = getCalendarLabels({ locale: "en-US", width: "short" });
  const labels = [enLabels.months[0], enLabels.months[11], enLabels.weekdays[0], enLabels.weekdays[6], enLabels.firstDayOfWeek,
    zhLabels.months[0], zhLabels.weekdays[0], zhLabels.firstDayOfWeek,
    deLabels.locale, deLabels.months[2], deLabels.weekdays[0], deLabels.firstDayOfWeek,
    narrow.months[0], narrow.weekdays[0], short.months[0], short.weekdays[0]];
  if (JSON.stringify(labels) !== JSON.stringify(["January", "December", "Monday", "Sunday", 7, "一月", "星期一", 1,
    "de-DE", "März", "Montag", 1, "J", "M", "Jan", "Mon"])) throw Error(JSON.stringify(labels));
  return renderApp(() => <text testId="formatted">formatted</text>, root);
}
`;

test("preview localizes date/time styles and standalone calendar labels", async t => {
  const work = await createTempWorkspace("preview-locale-format-");
  t.after(() => work.cleanup());
  const bundle = await buildFixtureApp(work.dir, "locale-format", formattingApp, {
    rawSource: true,
  });
  const { result } = await runScenario({
    projectDir: work.dir,
    bundle,
    name: "locale-format",
    profile: functionalProfile,
    requiredCapabilities: ["selectors.test-id", "tree.inspect"],
    steps: [
      {
        id: "localized",
        op: "assert",
        target: { surface: "main", testId: "formatted" },
        check: { property: "text", equals: "formatted" },
      },
    ],
  });
  assert.equal(result.teardown.clean, true);
});

test("preview naturally sorts a 10,000-item permutation and keeps collation ties stable", async t => {
  const work = await createTempWorkspace("preview-collation-");
  t.after(() => work.cleanup());
  const source = `
import { renderApp, type NativeNode } from "@hewel/zgui";
import { collatorSortIndices, compareStrings } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const values = Array.from({length: 10000}, (_, i) => "boot-" + ((i * 7919) % 10000));
  const indices = collatorSortIndices(values);
  if (!(indices instanceof Uint32Array) || indices.length !== values.length) {
    throw Error("expected one typed index per input");
  }
  for (let i = 0; i < indices.length; i++) {
    if (values[indices[i]!] !== "boot-" + i) throw Error("bad natural ordering at " + i);
  }
  const tied = collatorSortIndices(["boot-2", "BOOT-02", "boot-1"], {sensitivity: "base"});
  if (String(tied) !== "2,0,1") throw Error("collation ties lost input order");
  if (compareStrings("boot-2", "boot-10", {numeric: false}) <= 0 ||
      compareStrings("启动2", "启动10", {locale: "zh-CN"}) >= 0 ||
      compareStrings("a", "á", {sensitivity: "case"}) !== 0 ||
      compareStrings("a", "A", {sensitivity: "case"}) === 0 ||
      compareStrings("a", "á", {sensitivity: "accent"}) === 0) throw Error("collator options ignored");
  return renderApp(() => <text testId="order">{values[indices[2]!]} &lt; {values[indices[10]!]}</text>, root);
}
`;
  const bundle = await buildFixtureApp(work.dir, "locale-sort", source, { rawSource: true });
  await runScenario({
    projectDir: work.dir,
    bundle,
    name: "locale-sort",
    profile: functionalProfile,
    requiredCapabilities: ["selectors.test-id", "tree.inspect"],
    steps: [
      {
        id: "natural-order",
        op: "assert",
        target: { surface: "main", testId: "order" },
        check: { property: "text", equals: "boot-2 < boot-10" },
      },
    ],
  });
});

test("preview pins locale and zone while UI and isolated workers share web globals", async t => {
  const work = await createTempWorkspace("preview-locale-defaults-");
  t.after(() => work.cleanup());
  const source = `
import { createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { addZonedDuration, fromZonedDateTime, getSystemLocale, getSystemTimeZone, spawnWorker, toZonedDateTime } from "@hewel/zgui-services";
import { webGlobals } from "./web";
export const hostApiVersion = 1;

function App() {
  const zone = getSystemTimeZone();
  const locale = getSystemLocale();
  const hour = toZonedDateTime(0).hour;
  const ui = webGlobals();
  const [message, setMessage] = createSignal("pending");
  const worker = spawnWorker("web", {
    onMessage: value => setMessage(String(value)),
    onError: error => setMessage(error.code + ": " + error.message),
  });
  worker.postMessage(null);

  const timeZone = "America/New_York";
  const fold = {year: 2026, month: 11, day: 1, hour: 1, minute: 30};
  const early = fromZonedDateTime(fold, timeZone, {disambiguation: "earlier"});
  const late = fromZonedDateTime(fold, timeZone, {disambiguation: "later"});
  const beforeGap = fromZonedDateTime({year: 2026, month: 3, day: 7, hour: 2, minute: 30}, timeZone);
  const gap = addZonedDuration(beforeGap, {days: 1}, {timeZone, disambiguation: "later"});
  let rejected = false;
  try { fromZonedDateTime(fold, timeZone, {disambiguation: "reject"}); }
  catch (error) { if (!(error instanceof RangeError)) throw error; rejected = true; }
  if (late - early !== 3600000 || toZonedDateTime(gap, timeZone).hour !== 3 || !rejected) {
    throw Error("incorrect DST disambiguation");
  }

  return <column>
    <text testId="defaults">{zone + "/" + locale + "/" + hour}</text>
    <text testId="ui-globals">{ui}</text>
    <text testId="worker-globals">{message()}</text>
  </column>;
}
export function mount(root: NativeNode) { return renderApp(() => <App />, root); }
`;
  const bundle = await buildFixtureApp(work.dir, "locale-defaults", source, {
    rawSource: true,
    files: {
      "package.json": JSON.stringify({
        name: "locale-defaults",
        zgui: { workers: { web: "src/worker.ts" } },
      }),
      "src/worker.ts": `
import { webGlobals } from "./web";
declare function postMessage(value: string): void;
export function onMessage() { postMessage(webGlobals()); }
`,
      "src/web.ts": `
export function webGlobals(): string {
  const url = new URL("/p?q=one", "https://EXAMPLE.test");
  const params = new URLSearchParams(url.searchParams);
  params.set("q", "a + b");
  url.search = params.toString();
  url.searchParams.append("page", "2");
  const bytes = new TextEncoder().encode("中文");
  const decoder = new TextDecoder("utf8", {fatal: true});
  const first = decoder.decode(bytes.subarray(0, 2), {stream: true});
  const middle = decoder.decode(bytes.subarray(2, 4), {stream: true});
  return url.href + " | " + first + middle + decoder.decode(bytes.subarray(4));
}
`,
    },
  });
  const base = { schemaVersion: 1, id: "locale-fixture", services: {}, events: [] };
  const env = { TZ: "Pacific/Honolulu", LANG: "fr_FR.UTF-8", LC_ALL: "fr_FR.UTF-8" };
  const web = "https://example.test/p?q=a+%2B+b&page=2 | 中文";
  const run = (name, fixture, steps, allowFailure = false) =>
    runScenario({
      projectDir: work.dir,
      bundle,
      name,
      fixture,
      steps,
      profile: functionalProfile,
      requiredCapabilities: ["virtual-time", "selectors.test-id", "tree.inspect"],
      env,
      allowFailure,
    });
  const text = (testId, equals) => ({
    id: testId,
    op: "assert",
    target: { surface: "main", testId },
    check: { property: "text", equals },
  });
  for (const [name, fields, expected] of [
    ["ambient-ignored", {}, "UTC/en-US/0"],
    ["explicit-override", { locale: "ZH-cn", timeZone: "Asia/Shanghai" }, "Asia/Shanghai/zh-CN/8"],
  ]) {
    const fixture = path.join(work.dir, `${name}.fixture.json`);
    await writeJson(fixture, { ...base, ...fields });
    await run(name, fixture, [
      { id: "worker-turn", op: "clock.advance", byMs: 16 },
      text("defaults", expected),
      text("ui-globals", web),
      text("worker-globals", web),
    ]);
  }
  for (const [name, key, value, expectedError] of [
    ["bad-locale", "locale", "fr_FR.UTF-8", /locale/i],
    ["bad-zone", "timeZone", "Not/AZone", /time zone/i],
  ]) {
    const fixture = path.join(work.dir, `${name}.fixture.json`);
    await writeJson(fixture, { ...base, [key]: value });
    const { result } = await run(name, fixture, [{ id: "settle", op: "settle" }], true);
    assert.equal(result.firstError?.code, "E_SCHEMA");
    assert.match(result.firstError?.message, expectedError);
  }
});
