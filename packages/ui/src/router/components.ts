import { createContext, createMemo, getOwner, omit, onCleanup, useContext } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import { provide, withDefaults } from "../components/shared.js";
import type { Accessor, NativeChild, NativeElementProps, NativeEvent, NativeKeyEvent, NativeNode } from "../types.js";
import { buildPath, matchPattern, normalizePath } from "./path.js";
import type { ExtractRouteParams, SearchParams, SearchParamsInput } from "./path.js";
import type { BeforeLeaveGuard, NavigateOptions, RouteMatch, Router, RouterLocation, RouterNavigate } from "./core.js";

const RouterContext = createContext<Router<string> | null>(null);
const OutletDepthContext = createContext<number>(0);

function requireRouter(caller: string): Router<string> {
  const router = useContext(RouterContext);
  if (!router) {
    throw new ReferenceError(`${caller} requires an enclosing <RouterProvider>`);
  }
  return router;
}

/** Return the enclosing router instance (typed when a path union is supplied). */
export function useRouter<Paths extends string = string>(): Router<Paths> {
  return requireRouter("useRouter") as unknown as Router<Paths>;
}

/** Reactive current location (`pathname`, `search`, `searchParams`, `state`, `key`). */
export function useLocation<S = unknown>(): Accessor<RouterLocation<S>> {
  return requireRouter("useLocation").location as Accessor<RouterLocation<S>>;
}

/** Reactive route parameters for the active route match. */
export function useParams<P extends string = string>(): Accessor<
  string extends P ? Readonly<Record<string, string>> : ExtractRouteParams<P>
> {
  return requireRouter("useParams").params as Accessor<
    string extends P ? Readonly<Record<string, string>> : ExtractRouteParams<P>
  >;
}

/** Read and update query parameters on the active router. */
export function useSearchParams(): readonly [
  Accessor<SearchParams>,
  (
    next: SearchParamsInput | ((previous: SearchParams) => SearchParamsInput),
    options?: { readonly replace?: boolean | undefined },
  ) => Promise<boolean>,
] {
  const router = requireRouter("useSearchParams");
  const setSearchParams = (
    next: SearchParamsInput | ((previous: SearchParams) => SearchParamsInput),
    options?: { readonly replace?: boolean | undefined },
  ): Promise<boolean> => {
    const current = router.location();
    const resolved = typeof next === "function" ? next(current.searchParams) : next;
    return router.navigate(current.pathname, {
      search: resolved,
      replace: options?.replace ?? true,
      state: current.state,
    });
  };
  return [router.searchParams, setSearchParams] as const;
}

/** Return the enclosing router's `navigate` function. */
export function useNavigate<Paths extends string = string>(): RouterNavigate<Paths> {
  return requireRouter("useNavigate").navigate as unknown as RouterNavigate<Paths>;
}

/** Reactive match check against a route pattern on the enclosing router. */
export function useMatch<P extends string>(pattern: P, exact = true): Accessor<RouteMatch | null> {
  return requireRouter("useMatch").match(pattern, exact);
}

/** Register an owner-scoped navigation guard on the enclosing router. */
export function useBeforeLeave(guard: BeforeLeaveGuard): () => void {
  const router = requireRouter("useBeforeLeave");
  const dispose = router.onBeforeLeave(guard);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export interface RouterProviderProps<Paths extends string = string> {
  readonly router: Router<Paths>;
  readonly children?: NativeChild;
}

/** Provide a router instance to a subtree; renders `<Outlet />` directly when `children` is omitted. */
export function RouterProvider<Paths extends string = string>(props: RouterProviderProps<Paths>): NativeChild {
  const hasChildrenProp = "children" in props;
  return provide(RouterContext, props.router as unknown as Router<string>, () =>
    provide(OutletDepthContext, 0, () => {
      if (!hasChildrenProp) return Outlet();
      const children = props.children;
      return children === undefined ? Outlet() : children;
    }),
  );
}

/** Render the matched route component at the current nesting depth while preserving parent layouts. */
export function Outlet(): NativeChild {
  const router = requireRouter("Outlet");
  const depth = useContext(OutletDepthContext);

  // Skip path-only wrapper routes without components so layouts only wrap when present.
  const activeLayer = createMemo(() => {
    const list = router.matches();
    for (let i = depth; i < list.length; i++) {
      const layer = list[i]!;
      if (layer.route.component) {
        return { route: layer.route, nextDepth: i + 1 };
      }
    }
    return null;
  });

  // Memoize by route definition identity so sibling param changes keep the component mounted.
  const activeRoute = createMemo(() => activeLayer()?.route ?? null);
  const nextDepth = createMemo(() => activeLayer()?.nextDepth ?? depth + 1);

  return (() => {
    const route = activeRoute();
    const Component = route?.component;
    if (!Component) return null;
    const childDepth = nextDepth();
    return provide(OutletDepthContext, childDepth, () => Component({}));
  }) as unknown as NativeChild;
}

export interface LinkProps extends Omit<NativeElementProps, "role"> {
  readonly to: string;
  readonly params?: Readonly<Record<string, string>> | undefined;
  readonly search?: SearchParamsInput | undefined;
  readonly replace?: boolean | undefined;
  readonly state?: unknown;
  /** Require an exact pathname match for `data-active="true"` (defaults to `true`). */
  readonly exact?: boolean | undefined;
}

function runCallerHandler<E extends NativeEvent>(
  handler: ((event: E) => void) | null | undefined,
  event: E,
): boolean {
  if (!handler) return false;
  let cancelled = false;
  handler({ ...event, preventDefault() { event.preventDefault(); cancelled = true; } });
  return cancelled;
}

/** Declarative navigation trigger bound to the enclosing router. */
export function Link(props: LinkProps): NativeNode {
  const router = requireRouter("Link");
  const rest = omit(props, "to", "params", "search", "replace", "state", "exact", "onClick", "onKeyDown");

  const resolvedPathname = createMemo(() => {
    const raw = props.to;
    const base = raw.split("?", 1)[0] ?? "/";
    return base.includes(":") || base.includes("*")
      ? buildPath(base, props.params)
      : normalizePath(base);
  });

  const isActive = createMemo(() => {
    const exact = props.exact ?? true;
    return matchPattern(resolvedPathname(), router.location().pathname, exact) !== null;
  });

  const trigger = () => {
    if (props.disabled) return;
    const nav = router.navigate as (to: string, opts?: NavigateOptions) => Promise<boolean>;
    void nav(props.to, {
      params: props.params,
      search: props.search,
      replace: props.replace,
      state: props.state,
    });
  };

  const node = createElement("box");
  spread(node, mergeProps(
    rest,
    withDefaults({
      role: "link" as const,
      get tabIndex() { return props.disabled ? -1 : 0; },
      get "data-active"() { return isActive() ? "true" : "false"; },
    }, rest),
    {
      onClick(event: NativeEvent) {
        if (!runCallerHandler(props.onClick, event)) trigger();
      },
      onKeyDown(event: NativeKeyEvent) {
        if (runCallerHandler(props.onKeyDown, event) || props.disabled) return;
        if (event.key === "Enter" || event.key === " " || event.key === "Space") {
          event.preventDefault();
          trigger();
        }
      },
    },
  ));
  return node;
}
