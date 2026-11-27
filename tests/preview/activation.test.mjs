import assert from "node:assert/strict";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  functionalProfile,
  runScenario,
  writeJson,
} from "./harness.mjs";

const source = version => `
import { Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { getInitialActivation, onActivation } from "@hewel/zgui-services";
export const hostApiVersion = 1;
try {
  getInitialActivation();
  throw new Error("module evaluation unexpectedly read the activation");
} catch (error) {
  if (!(error instanceof TypeError)) throw error;
}
export function mount(root: NativeNode) {
  const describe = (info: ReturnType<typeof getInitialActivation>) =>
    info.args.join("|") + " @ " + info.cwd;
  return renderApp(() => {
    const [events, setEvents] = createSignal("${version}");
    const [visible, setVisible] = createSignal(true);
    onActivation(info => setEvents(previous => previous + ";" + describe(info)));
    const stop = onActivation(() => { throw new Error("unsubscribed handler ran"); });
    stop();
    stop();
    const Child = () => {
      onActivation(() => { throw new Error("disposed owner received activation"); });
      return <text>Listening</text>;
    };
    ${version === "reject" ? 'throw new Error("candidate rejected");' : ""}
    return <column>
      <text testId="cold">{describe(getInitialActivation())}</text>
      <text testId="events">{events()}</text>
      <control testId="hide" onClick={() => setVisible(false)}>Hide</control>
      <Show when={visible()}><Child /></Show>
    </column>;
  }, root);
}
`;

test("preview delivers fixture activation only to subscribed owners of the accepted generation", async t => {
  const work = await createTempWorkspace("preview-activation-");
  t.after(() => work.cleanup());
  const build = version =>
    buildFixtureApp(path.join(work.dir, version), "activation", source(version), {
      rawSource: true,
      development: true,
      capabilities: { singleInstance: true },
    });
  const first = await build("first");
  const rejected = await build("reject");
  const second = await build("second");
  const fixture = path.join(work.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "activation",
    services: {},
    events: [],
    activation: { args: ["--boot", "-1", "two words"], cwd: "/cold" },
  });
  let step = 0;
  const add = action => ({ id: `step-${++step}`, ...action });
  const text = (testId, equals) =>
    add({ op: "assert", target: { surface: "main", testId }, check: { property: "text", equals } });
  const hide = () =>
    add({ op: "pointer.click", button: "primary", target: { surface: "main", testId: "hide" } });
  const emit = (args, cwd) =>
    add({ op: "service.emit", service: "activation", value: { args, cwd } });
  const reload = (bundle, expect) => add({ op: "lifecycle.reload", bundle, expect });
  const { result, out } = await runScenario({
    projectDir: work.dir,
    bundle: first,
    fixture,
    profile: functionalProfile,
    name: "activation",
    requiredCapabilities: [
      "selectors.test-id",
      "input.pointer",
      "services.fixtures",
      "lifecycle.reload",
    ],
    steps: [
      text("cold", "--boot|-1|two words @ /cold"),
      hide(),
      emit(["log α"], "/live"),
      text("events", "first;log α @ /live"),
      reload(rejected, "rejected"),
      emit(["after rejection"], null),
      text("events", "first;log α @ /live;after rejection @ null"),
      reload(second, "accepted"),
      text("cold", "--boot|-1|two words @ /cold"),
      hide(),
      emit(["after acceptance"], "/next"),
      text("events", "second;after acceptance @ /next"),
    ],
  });
  assert.equal(result.teardown.clean, true);
  const { records } = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
  assert.deepEqual(
    records
      .filter(record => record.kind === "delivery" && record.detail.service === "activation")
      .map(record => record.detail.generation),
    [1, 1, 3],
  );
});
