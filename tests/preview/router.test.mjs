import assert from "node:assert/strict";
import { test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  runScenario,
} from "./harness.mjs";

const routerPreviewSource = `
import { Button, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Link,
  Outlet,
  RouterProvider,
  createRouter,
  useBeforeLeave,
  useLocation,
  useParams,
  useRouter,
} from "@hewel/zgui/router";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dirty, setDirty] = createSignal(false);
    const [vetoCount, setVetoCount] = createSignal(0);
    let layoutMountCount = 0;

    function MainLayout() {
      layoutMountCount += 1;
      const router = useRouter();
      const loc = useLocation();
      return (
        <column style="gap: 6px; padding: 8px; border: 1px solid #555; flex: 1">
          <text testId="main-layout-mounts">{String(layoutMountCount)}</text>
          <text testId="main-path">{loc().pathname + loc().search}</text>
          <row style="gap: 6px">
            <Link testId="nav-home" to="/">Home</Link>
            <Link testId="nav-doc-1" to="/docs/:id" params={{ id: "readme" }}>Readme</Link>
            <Link testId="nav-doc-2" to="/docs/:id" params={{ id: "guide" }} search={{ tab: "api" }}>Guide</Link>
            <Button testId="btn-back" onClick={() => { void router.back(); }}>Back</Button>
            <Button testId="btn-forward" onClick={() => { void router.forward(); }}>Forward</Button>
          </row>
          <Outlet />
        </column>
      );
    }

    function HomeView() {
      return <text testId="main-view">HOME</text>;
    }

    function DocView() {
      const params = useParams<"/docs/:id">();
      useBeforeLeave(event => {
        if (dirty()) {
          setVetoCount(n => n + 1);
          event.preventDefault();
        }
      });
      return (
        <column style="gap: 4px">
          <text testId="main-view">{"DOC:" + params().id}</text>
          <Button testId="btn-toggle-dirty" onClick={() => setDirty(v => !v)}>
            {dirty() ? "Dirty:ON" : "Dirty:OFF"}
          </Button>
        </column>
      );
    }

    function SideView() {
      const router = useRouter();
      const loc = useLocation();
      return (
        <column style="gap: 6px; padding: 8px; border: 1px solid #555; width: 220px">
          <text testId="side-path">{loc().pathname}</text>
          <Button testId="btn-side-next" onClick={() => { void router.navigate("/inspect/42"); }}>Inspect</Button>
          <Outlet />
        </column>
      );
    }

    const mainRouter = createRouter([
      {
        path: "/",
        component: MainLayout,
        children: [
          { path: "", component: HomeView },
          { path: "docs/:id", component: DocView },
        ],
      },
    ] as const, { initialPath: "/" });

    const sideRouter = createRouter([
      {
        path: "/",
        component: SideView,
        children: [
          { path: "", component: () => <text testId="side-view">SIDE:ROOT</text> },
          { path: "inspect/:item", component: () => {
            const p = useParams<"/inspect/:item">();
            return <text testId="side-view">{"SIDE:" + p().item}</text>;
          } },
        ],
      },
    ] as const, { initialPath: "/" });

    return (
      <column style="width: 100%; height: 100%; padding: 8px; gap: 8px">
        <text testId="veto-count">{String(vetoCount())}</text>
        <row style="gap: 8px; flex: 1">
          <RouterProvider router={mainRouter} />
          <RouterProvider router={sideRouter} />
        </row>
      </column>
    );
  }, root);
}
`;

test("@hewel/zgui/router multi-instance routers, nested Outlet, back/forward, and useBeforeLeave veto work in preview (#57)", async t => {
  const temp = await createTempWorkspace("preview-router-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "router-app", routerPreviewSource, {
    rawSource: true,
  });

  const click = (id, testId) => ({
    id,
    op: "pointer.click",
    button: "primary",
    target: { surface: "main", testId },
  });
  const check = (id, testId, equals) => ({
    id,
    op: "assert",
    target: { surface: "main", testId },
    check: { property: "text", equals },
  });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "router-multi-instance",
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"],
    width: 720,
    height: 320,
    scale: 1,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      check("init-main", "main-view", "HOME"),
      check("init-side", "side-view", "SIDE:ROOT"),
      check("init-mounts", "main-layout-mounts", "1"),

      // Navigate main router via <Link>
      click("go-doc-1", "nav-doc-1"),
      check("after-doc-1", "main-view", "DOC:readme"),
      check("after-doc-1-path", "main-path", "/docs/readme"),
      check("mounts-still-1", "main-layout-mounts", "1"),

      // Turn dirty guard ON -> navigating to doc-2 or Home must be vetoed
      click("dirty-on", "btn-toggle-dirty"),
      click("blocked-doc-2", "nav-doc-2"),
      check("still-doc-1", "main-view", "DOC:readme"),
      check("veto-1", "veto-count", "1"),

      // Side router still navigates independently while main router is dirty!
      click("side-nav", "btn-side-next"),
      check("side-moved", "side-view", "SIDE:42"),
      check("main-unaffected", "main-view", "DOC:readme"),

      // Turn dirty guard OFF -> navigate to doc-2, then back() and forward()
      click("dirty-off", "btn-toggle-dirty"),
      click("go-doc-2", "nav-doc-2"),
      check("at-doc-2", "main-view", "DOC:guide"),
      check("at-doc-2-path", "main-path", "/docs/guide?tab=api"),
      click("go-back", "btn-back"),
      check("back-at-doc-1", "main-view", "DOC:readme"),
      click("go-forward", "btn-forward"),
      check("forward-at-doc-2", "main-view", "DOC:guide"),
      check("mounts-still-1-end", "main-layout-mounts", "1"),
      { id: "cap-final", op: "capture", captureId: "final" },
    ],
  });

  assert.equal(result.teardown?.clean, true);
});
