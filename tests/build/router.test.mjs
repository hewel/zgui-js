import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const routerFixture = `
import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
import {
  Link,
  Outlet,
  RouterProvider,
  bindControlNavigation,
  createRouter,
  useBeforeLeave,
  useLocation,
  useMatch,
  useParams,
  useSearchParams,
} from "@hewel/zgui/router";

export const counters = {
  layoutMounts: 0,
  docMounts: 0,
  docCleanups: 0,
  guardCalls: 0,
  controlUnsubscribed: 0,
};

const [dirty, setDirty] = createSignal(false);
let asyncResolver = null;
let asyncMode = false;

export { setDirty };
export function setAsyncGuard(enabled) { asyncMode = enabled; }
export function resolveAsyncGuard(allow) {
  const fn = asyncResolver;
  asyncResolver = null;
  fn?.(allow);
}

function RootLayout() {
  counters.layoutMounts += 1;
  const [layoutLocal, setLayoutLocal] = createSignal("kept");
  const loc = useLocation();
  const isDocs = useMatch("/docs/:id");
  return (
    <column testId="layout">
      <text testId="layout-state">{layoutLocal()}</text>
      <text testId="layout-path">{loc().pathname + loc().search}</text>
      <text testId="layout-docs-match">{isDocs() ? isDocs().params.id : "none"}</text>
      <Link testId="link-home" to="/">Home</Link>
      <Link testId="link-doc-b" to="/docs/:id" params={{ id: "beta" }} search={{ tab: "preview" }}>Doc Beta</Link>
      <Link testId="link-disabled" to="/settings" disabled>Disabled</Link>
      <Outlet />
    </column>
  );
}

function HomePage() {
  return <text testId="page-home">Home Screen</text>;
}

function DocPage() {
  counters.docMounts += 1;
  onCleanup(() => { counters.docCleanups += 1; });
  const params = useParams();
  const [search, setSearch] = useSearchParams();
  useBeforeLeave(event => {
    counters.guardCalls += 1;
    if (asyncMode) {
      return new Promise(resolve => { asyncResolver = resolve; });
    }
    if (dirty()) {
      event.preventDefault();
      return;
    }
  });
  globalThis.__setSearch = setSearch;
  return <text testId="page-doc">{params().id + "|" + (search().tab ?? "")}</text>;
}

function SettingsPage() {
  return <text testId="page-settings">Settings</text>;
}

function NotFoundPage() {
  const params = useParams();
  return <text testId="page-404">{"404:" + (params()["*"] ?? "")}</text>;
}

const routes = [
  {
    path: "/",
    component: RootLayout,
    children: [
      { path: "", component: HomePage },
      { path: "docs/:id", component: DocPage },
      { path: "settings", component: SettingsPage },
      { path: "*", component: NotFoundPage },
    ],
  },
];

export let leftRouter;
export let rightRouter;
let controlEmit = null;
export function emitControl(msg) { controlEmit?.(msg); }

export function mount(root) {
  return renderApp(() => {
    leftRouter = createRouter(routes, { initialPath: "/" });
    rightRouter = createRouter([
      { path: "/", component: () => <text testId="right-home">Right Home</text> },
      { path: "/item/:slug", component: () => {
        const p = useParams();
        return <text testId="right-item">{"Item:" + p().slug}</text>;
      } },
    ], { initialPath: "/item/one" });

    bindControlNavigation(leftRouter, {
      subscribe(handler) {
        controlEmit = handler;
        return () => {
          counters.controlUnsubscribed += 1;
          controlEmit = null;
        };
      },
    });

    return (
      <row>
        <RouterProvider router={leftRouter} />
        <RouterProvider router={rightRouter} />
      </row>
    );
  }, root);
}
`;

test("@hewel/zgui/router supports nested Outlets, multi-instance history, Link, search params, sync/async guards, and control bridge (#57)", async t => {
  const h = createHost();
  const mod = await moduleFor(t, routerFixture);
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
      delete globalThis.__setSearch;
    }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));

  const textOf = testId => h.host.getTextContent(h.byTestId(testId));

  // Initial mount: both routers render independently.
  assert.equal(textOf("page-home"), "Home Screen");
  assert.equal(textOf("right-item"), "Item:one");
  assert.equal(mod.counters.layoutMounts, 1);
  assert.equal(mod.leftRouter.canGoBack(), false);
  assert.equal(mod.leftRouter.canGoForward(), false);
  assert.equal(h.byTestId("link-home").props.get("data-active"), "true");

  // Navigate left router to parameterized doc route; right router is unaffected.
  await h.runTurn(() =>
    mod.leftRouter.navigate("/docs/:id", {
      params: { id: "alpha" },
      search: { tab: "edit" },
      state: { source: "toc" },
    }),
  );
  assert.equal(textOf("page-doc"), "alpha|edit");
  assert.equal(textOf("layout-docs-match"), "alpha");
  assert.equal(textOf("right-item"), "Item:one");
  assert.equal(mod.counters.layoutMounts, 1, "parent layout stays mounted across child navigation");
  assert.equal(mod.counters.docMounts, 1);
  assert.equal(mod.leftRouter.canGoBack(), true);
  assert.deepEqual(mod.leftRouter.location().state, { source: "toc" });

  // Sibling param change keeps DocPage mounted and updates params reactively.
  await h.runTurn(() => mod.leftRouter.navigate("/docs/beta?tab=split"));
  assert.equal(textOf("page-doc"), "beta|split");
  assert.equal(mod.counters.docMounts, 1, "same route definition stays mounted on param change");
  assert.equal(mod.leftRouter.entries().length, 3);

  // useSearchParams updates query in place (replace default).
  await h.runTurn(() => globalThis.__setSearch({ tab: "preview", q: "中 文" }));
  assert.equal(textOf("page-doc"), "beta|preview");
  assert.equal(mod.leftRouter.searchParams().q, "中 文");
  assert.equal(mod.leftRouter.entries().length, 3, "useSearchParams defaults to replace");

  // Synchronous useBeforeLeave veto blocks both navigate() and back().
  h.runTurn(() => mod.setDirty(true));
  const blockedPush = await h.runTurn(() => mod.leftRouter.navigate("/settings"));
  assert.equal(blockedPush, false);
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");
  const blockedBack = await h.runTurn(() => mod.leftRouter.back());
  assert.equal(blockedBack, false);
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");

  // Clear dirty flag -> back() and forward() traverse history cleanly.
  h.runTurn(() => mod.setDirty(false));
  assert.equal(await h.runTurn(() => mod.leftRouter.back()), true);
  assert.equal(mod.leftRouter.location().pathname, "/docs/alpha");
  assert.equal(mod.leftRouter.canGoForward(), true);
  assert.equal(await h.runTurn(() => mod.leftRouter.forward()), true);
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");

  // Async guard: isNavigating flips true while pending and resolves cleanly.
  mod.setAsyncGuard(true);
  const pendingNav = h.runTurn(() => mod.leftRouter.navigate("/settings"));
  assert.equal(mod.leftRouter.isNavigating(), true);
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");
  mod.resolveAsyncGuard(true);
  assert.equal(await pendingNav, true);
  assert.equal(mod.leftRouter.isNavigating(), false);
  assert.equal(textOf("page-settings"), "Settings");
  assert.equal(mod.counters.docCleanups, 1, "leaving DocPage cleaned up its owner & guard");
  mod.setAsyncGuard(false);

  // Leaving DocPage removed its guard (no further guardCalls when navigating away from Settings).
  const callsBefore = mod.counters.guardCalls;
  await h.runTurn(() => mod.leftRouter.navigate("/unknown/nested/path"));
  assert.equal(mod.counters.guardCalls, callsBefore, "unmounted route guard must not fire");
  assert.equal(textOf("page-404"), "404:unknown/nested/path");

  // Link click and keyboard Enter activate navigation; disabled Link does nothing.
  h.runTurn(() => h.byTestId("link-disabled").callbacks.onClick({ preventDefault() {} }));
  assert.equal(mod.leftRouter.location().pathname, "/unknown/nested/path");
  h.runTurn(() => h.byTestId("link-doc-b").callbacks.onClick({ preventDefault() {} }));
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");
  assert.equal(mod.leftRouter.searchParams().tab, "preview");

  // bindControlNavigation routes host control messages cleanly.
  h.runTurn(() => mod.emitControl({ command: "other", payload: "/settings" }));
  assert.equal(mod.leftRouter.location().pathname, "/docs/beta");
  h.runTurn(() =>
    mod.emitControl({ command: "navigate", payload: { path: "/settings", replace: true } }),
  );
  assert.equal(mod.leftRouter.location().pathname, "/settings");

  // Independent right router navigates without touching left router.
  await h.runTurn(() => mod.rightRouter.navigate("/"));
  assert.equal(textOf("right-home"), "Right Home");
  assert.equal(mod.leftRouter.location().pathname, "/settings");

  // Disposing root unsubscribes control navigation cleanly.
  h.runTurn(() => dispose());
  dispose = undefined;
  assert.equal(mod.counters.controlUnsubscribed, 1);
});
