import {
  Link,
  Outlet,
  RouterProvider,
  bindControlNavigation,
  createRouter,
  useBeforeLeave,
  useLocation,
  useMatch,
  useNavigate,
  useParams,
  useRouter,
  useSearchParams,
} from "@hewel/zgui/router";
import type {
  BeforeLeaveEvent,
  ExtractRouteParams,
  RouteDefinition,
  RoutePaths,
  Router,
} from "@hewel/zgui/router";

const RootLayout = () => (
  <column>
    <Outlet />
  </column>
);
const HomePage = () => <text>home</text>;
const DocPage = () => {
  const params = useParams<"/docs/:docId">();
  const [search, setSearch] = useSearchParams();
  const loc = useLocation<{ readonly fromOutline?: boolean }>();
  const matched = useMatch("/docs/:docId");
  const navigate = useNavigate<AppPaths>();
  useBeforeLeave((event: BeforeLeaveEvent) => {
    if (event.to.pathname === "/blocked") return false;
  });
  void setSearch({ tab: "preview", line: 12 }, { replace: true });
  void navigate("/settings/:section", { params: { section: "theme" } });
  return (
    <text>
      {params().docId}:{search().tab}:{String(loc().state?.fromOutline)}:{matched()?.pathname}
    </text>
  );
};
const SettingsPage = () => <text>settings</text>;
const NotFound = () => <text>404</text>;

const routes = [
  {
    path: "/",
    component: RootLayout,
    children: [
      { path: "", component: HomePage },
      { path: "docs/:docId", component: DocPage },
      { path: "settings/:section", component: SettingsPage },
      { path: "*", component: NotFound },
    ],
  },
] as const satisfies readonly RouteDefinition[];

type AppPaths = RoutePaths<typeof routes>;
type AssertEqual<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;

export type InferredPathsCheck = AssertEqual<
  AppPaths,
  "/" | "/docs/:docId" | "/settings/:section" | "/*"
>;
export type DocParamsCheck = AssertEqual<
  ExtractRouteParams<"/docs/:docId">,
  { readonly docId: string }
>;
export type WildcardParamsCheck = AssertEqual<ExtractRouteParams<"/*">, { readonly "*": string }>;
export type StaticParamsCheck = AssertEqual<
  ExtractRouteParams<"/">,
  Readonly<Record<string, never>>
>;

const router: Router<AppPaths> = createRouter(routes, { initialPath: "/" });
void router.navigate("/");
void router.navigate("/docs/:docId", { params: { docId: "intro" }, search: { mode: "split" } });
void router.navigate("/docs/intro", { replace: true });
void router.back();
void router.forward();
void router.go(-1);

// @ts-expect-error parameterized pattern requires params object
void router.navigate("/docs/:docId");
// @ts-expect-error wrong param key is rejected
void router.navigate("/docs/:docId", { params: { wrong: "1" } });

const stopControl = bindControlNavigation(router, {
  command: "navigate",
  subscribe: handler => {
    handler({ command: "navigate", payload: { path: "/docs/readme" } });
    return () => {};
  },
});
stopControl();

const InRouter = () => {
  const active = useRouter<AppPaths>();
  void active.canGoBack();
  return (
    <Link to="/docs/:docId" params={{ docId: "guide" }} search={{ q: "test" }}>
      Guide
    </Link>
  );
};

void (
  <RouterProvider router={router}>
    <InRouter />
    <Outlet />
  </RouterProvider>
);
void (<RouterProvider router={router} />);
