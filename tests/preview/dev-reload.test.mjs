import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  runScenario,
} from "./harness.mjs";

function makeAppSource({ banner, version = 1, throwInMount = false }) {
  return `
import { Button, createDevState, renderApp, type NativeNode } from "@hewel/zgui";
import { createRouter, Outlet, RouterProvider, useLocation, useRouter } from "@hewel/zgui/router";

export const hostApiVersion = 1;

function Page(props: { name: string }) {
  return <label testId="route-page">{props.name}</label>;
}

function Shell(props: { banner: string }) {
  const router = useRouter();
  const loc = useLocation();
  const [count, setCount] = createDevState<number>("counter", 0, { version: ${version} });
  const [note, setNote] = createDevState<string>("note", "init", { version: ${version} });
  const [rejectMsg, setRejectMsg] = createDevState<string>("rejectMsg", "ok", { version: ${version} });

  return (
    <column style={{ width: "100%", height: "100%", padding: "16px", gap: "8px" }}>
      <label testId="banner">{props.banner}</label>
      <label testId="counter-val">{\`count:\${count()}\`}</label>
      <label testId="note-val">{\`note:\${note()}\`}</label>
      <label testId="reject-val">{rejectMsg()}</label>
      <label testId="router-loc">{\`\${loc().pathname}|\${router.canGoBack()}|\${router.canGoForward()}\`}</label>
      <Outlet />
      <row style={{ gap: "8px" }}>
        <Button testId="btn-inc" onClick={() => setCount(c => c + 1)}>Inc</Button>
        <Button testId="btn-note" onClick={() => setNote("saved-draft")}>Note</Button>
        <Button testId="btn-bad-state" onClick={() => {
          try {
            (setNote as unknown as (v: unknown) => void)({ fn: () => "closure" });
            setRejectMsg("accepted-bad");
          } catch {
            try {
              (setNote as unknown as (v: unknown) => void)(Promise.resolve(1));
              setRejectMsg("accepted-promise");
            } catch {
              setRejectMsg("rejected-non-plain");
            }
          }
        }}>Bad</Button>
        <Button testId="nav-docs" onClick={() => { void router.navigate("/docs/wave2"); }}>Docs</Button>
        <Button testId="nav-settings" onClick={() => { void router.navigate("/settings"); }}>Settings</Button>
        <Button testId="nav-back" onClick={() => { void router.back(); }}>Back</Button>
        <Button testId="nav-forward" onClick={() => { void router.forward(); }}>Forward</Button>
      </row>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  ${throwInMount ? 'throw new Error("intentional candidate mount failure");' : ""}
  const router = createRouter([
    { path: "/home", component: () => <Page name="Home" /> },
    { path: "/docs/:slug", component: () => <Page name="Docs" /> },
    { path: "/settings", component: () => <Page name="Settings" /> },
  ], { initialPath: "/home" });
  return renderApp(() => (
    <RouterProvider router={router}>
      <Shell banner="${banner}" />
    </RouterProvider>
  ), root);
}
`;
}

let serial = 0;
const id = () => `s-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const reload = (bundle, expect = "accepted") => ({ id: id(), op: "lifecycle.reload", bundle, expect });

const CAPS = [
  "pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect", "lifecycle.reload",
];

test("dev reload preserves createDevState + router stack, rolls back rejected candidates, and cold-starts on schema/capability mismatch", { timeout: 60000 }, async t => {
  const temp = await createTempWorkspace("preview-dev-reload-");
  t.after(() => temp.cleanup());

  const v1 = await buildFixtureApp(path.join(temp.dir, "v1"), "dev-reload-app", makeAppSource({ banner: "v1" }), { rawSource: true, development: true });
  const v2Bad = await buildFixtureApp(path.join(temp.dir, "v2-bad"), "dev-reload-app", makeAppSource({ banner: "v2-bad", throwInMount: true }), { rawSource: true, development: true });
  const v2Good = await buildFixtureApp(path.join(temp.dir, "v2-good"), "dev-reload-app", makeAppSource({ banner: "v2-good" }), { rawSource: true, development: true });
  const v3VersionBump = await buildFixtureApp(path.join(temp.dir, "v3-ver"), "dev-reload-app", makeAppSource({ banner: "v3-ver", version: 2 }), { rawSource: true, development: true });
  const v4CapChange = await buildFixtureApp(
    path.join(temp.dir, "v4-cap"),
    "dev-reload-app",
    makeAppSource({ banner: "v4-cap", version: 2 }),
    { rawSource: true, development: true, capabilities: { clipboard: { read: true } } },
  );

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle: v1,
    name: "dev-reload-flow",
    id: "dev-reload-flow",
    requiredCapabilities: CAPS,
    steps: [
      advance(40),
      check("banner", "text", "v1"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),

      // Mutate dev state & navigate history stack
      click("btn-inc"),
      click("btn-inc"),
      click("btn-inc"),
      click("btn-note"),
      click("btn-bad-state"),
      click("nav-docs"),
      click("nav-settings"),
      click("nav-back"),
      advance(20),
      check("counter-val", "text", "count:3"),
      check("note-val", "text", "note:saved-draft"),
      check("reject-val", "text", "rejected-non-plain"),
      check("router-loc", "text", "/docs/wave2|true|true"),

      // 1. Rejected reload rolls back cleanly; predecessor stays mounted and interactive
      reload(v2Bad, "rejected"),
      advance(20),
      check("banner", "text", "v1"),
      click("btn-inc"),
      advance(20),
      check("counter-val", "text", "count:4"),
      check("router-loc", "text", "/docs/wave2|true|true"),

      // 2. Accepted reload preserves counter, note, AND router location + history stack
      reload(v2Good, "accepted"),
      advance(40),
      check("banner", "text", "v2-good"),
      check("counter-val", "text", "count:4"),
      check("note-val", "text", "note:saved-draft"),
      check("router-loc", "text", "/docs/wave2|true|true"),
      // Forward still reaches /settings from preserved history stack!
      click("nav-forward"),
      advance(20),
      check("router-loc", "text", "/settings|true|false"),

      // 3. Entry version bump resets createDevState slots while router stays preserved
      reload(v3VersionBump, "accepted"),
      advance(40),
      check("banner", "text", "v3-ver"),
      check("counter-val", "text", "count:0"),
      check("note-val", "text", "note:init"),
      check("router-loc", "text", "/settings|true|false"),

      // 4. Capability digest mismatch forces full cold start (including router)
      click("btn-inc"),
      advance(20),
      check("counter-val", "text", "count:1"),
      reload(v4CapChange, "accepted"),
      advance(40),
      check("banner", "text", "v4-cap"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),
    ],
  });

  assert.equal(result.teardown?.clean, true);
});

test("production build degrades createDevState and router dev preservation to plain no-ops", { timeout: 45000 }, async t => {
  const temp = await createTempWorkspace("preview-prod-reload-");
  t.after(() => temp.cleanup());

  const prodV1 = await buildFixtureApp(path.join(temp.dir, "prod-v1"), "prod-reload-app", makeAppSource({ banner: "prod-v1" }), { rawSource: true, development: false });
  const prodV2 = await buildFixtureApp(path.join(temp.dir, "prod-v2"), "prod-reload-app", makeAppSource({ banner: "prod-v2" }), { rawSource: true, development: false });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle: prodV1,
    name: "prod-reload-noop",
    id: "prod-reload-noop",
    requiredCapabilities: CAPS,
    steps: [
      advance(40),
      click("btn-inc"),
      click("btn-inc"),
      click("nav-docs"),
      advance(20),
      check("counter-val", "text", "count:2"),
      check("router-loc", "text", "/docs/wave2|true|false"),
      reload(prodV2, "accepted"),
      advance(40),
      check("banner", "text", "prod-v2"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),
    ],
  });

  assert.equal(result.teardown?.clean, true);
});
