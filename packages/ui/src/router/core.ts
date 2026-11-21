import { createMemo, createSignal, getOwner, onCleanup, untrack } from "#solid";
import { getDevStateChannel, trySerializePlainDevState } from "../dev-state.js";
import { flush } from "../renderer/index.js";
import type { Accessor, Component } from "../types.js";
import {
  buildPath,
  joinPaths,
  matchPattern,
  normalizePath,
  normalizeSearchInput,
  serializeSearch,
  splitPathAndSearch,
} from "./path.js";
import type {
  ConcretePath,
  ExtractRouteParams,
  HasRouteParams,
  JoinPaths,
  SearchParams,
  SearchParamsInput,
} from "./path.js";

/** Route definition node inside a typed route tree. */
export interface RouteDefinition<P extends string = string> {
  readonly path: P;
  readonly component?: Component<{}> | undefined;
  readonly children?: readonly RouteDefinition[] | undefined;
  readonly meta?: Readonly<Record<string, unknown>> | undefined;
}

/** Recursively extract every full normalized route pattern declared in `Routes`. */
export type RoutePaths<
  Routes extends readonly RouteDefinition[],
  Prefix extends string = "",
> = Routes extends readonly [infer Head extends RouteDefinition, ...infer Tail extends readonly RouteDefinition[]]
  ? (Head["children"] extends readonly RouteDefinition[]
      ? JoinPaths<Prefix, Head["path"]> | RoutePaths<Head["children"], JoinPaths<Prefix, Head["path"]>>
      : JoinPaths<Prefix, Head["path"]>)
    | RoutePaths<Tail, Prefix>
  : never;

/** Snapshot of one entry in a router's in-memory history stack. */
export interface RouterLocation<S = unknown> {
  readonly key: string;
  readonly pathname: string;
  readonly search: string;
  readonly searchParams: SearchParams;
  readonly state: S;
}

/** One matched layer in the active nested route chain (root -> leaf). */
export interface RouteMatch {
  readonly route: RouteDefinition;
  readonly pattern: string;
  readonly pathname: string;
  readonly params: Readonly<Record<string, string>>;
}

/** Event passed to `useBeforeLeave` / `router.onBeforeLeave` guards. */
export interface BeforeLeaveEvent {
  readonly from: RouterLocation;
  readonly to: RouterLocation;
  readonly delta: number | null;
  readonly replace: boolean;
  readonly defaultPrevented: boolean;
  preventDefault(): void;
}

/** Sync or async guard that vetoes navigation when returning `false` or calling `preventDefault()`. */
export type BeforeLeaveGuard = (event: BeforeLeaveEvent) => boolean | void | Promise<boolean | void>;

/** Options for `router.navigate(to, options)`. */
export interface NavigateOptions<P extends string = string, S = unknown> {
  readonly params?: ExtractRouteParams<P> | Readonly<Record<string, string>> | undefined;
  readonly search?: SearchParamsInput | undefined;
  readonly replace?: boolean | undefined;
  readonly state?: S | undefined;
}

/** Allowed targets for typed `navigate(...)` calls. */
export type NavigateTarget<Paths extends string> =
  Paths | ConcretePath<Paths> | (string & {});

/** Strict `navigate` signature requiring `params` when navigating by parameterized route pattern. */
export type RouterNavigate<Paths extends string = string> = <To extends NavigateTarget<Paths>, S = unknown>(
  to: To,
  ...rest: To extends Paths
    ? HasRouteParams<To> extends true
      ? [options: Omit<NavigateOptions<To, S>, "params"> & { readonly params: ExtractRouteParams<To> }]
      : [options?: NavigateOptions<To, S>]
    : [options?: NavigateOptions<string, S>]
) => Promise<boolean>;

/** Options passed to `createRouter(routes, options)`. */
export interface CreateRouterOptions<Paths extends string = string, S = unknown> {
  readonly initialPath?: NavigateTarget<Paths> | undefined;
  readonly initialEntries?: readonly string[] | undefined;
  readonly initialIndex?: number | undefined;
  readonly initialState?: S | undefined;
  /**
   * Key used to persist active location and history stack across dev reloads.
   * Defaults to `"__zgui_router"`; set to `false` to opt out. Ignored in production builds.
   */
  readonly devStateKey?: string | false | undefined;
}

/** Independent, owner-friendly in-memory router instance. */
export interface Router<Paths extends string = string> {
  readonly routes: readonly RouteDefinition[];
  readonly entries: Accessor<readonly RouterLocation[]>;
  readonly index: Accessor<number>;
  readonly location: Accessor<RouterLocation>;
  readonly matches: Accessor<readonly RouteMatch[]>;
  readonly params: Accessor<Readonly<Record<string, string>>>;
  readonly searchParams: Accessor<SearchParams>;
  readonly canGoBack: Accessor<boolean>;
  readonly canGoForward: Accessor<boolean>;
  readonly isNavigating: Accessor<boolean>;
  readonly navigate: RouterNavigate<Paths>;
  back(): Promise<boolean>;
  forward(): Promise<boolean>;
  go(delta: number): Promise<boolean>;
  match<P extends Paths | (string & {})>(pattern: P, exact?: boolean): Accessor<RouteMatch | null>;
  onBeforeLeave(guard: BeforeLeaveGuard): () => void;
  dispose(): void;
}

interface CompiledBranch {
  readonly leafPattern: string;
  readonly chain: readonly { readonly route: RouteDefinition; readonly pattern: string }[];
  readonly order: number;
}

function compileBranches(
  routes: readonly RouteDefinition[],
  prefix: string,
  ancestors: readonly { readonly route: RouteDefinition; readonly pattern: string }[],
  out: CompiledBranch[],
): void {
  for (const route of routes) {
    const pattern = joinPaths(prefix, route.path);
    const chain = [...ancestors, { route, pattern }];
    if (route.children && route.children.length > 0) {
      compileBranches(route.children, pattern, chain, out);
      // A layout route without an explicit index child still matches its own path if it has a component.
      const hasIndex = route.children.some(child => child.path === "" || child.path === "/");
      if (route.component && !hasIndex) {
        out.push({ leafPattern: pattern, chain, order: out.length });
      }
    } else {
      out.push({ leafPattern: pattern, chain, order: out.length });
    }
  }
}

function resolveMatches(branches: readonly CompiledBranch[], pathname: string): readonly RouteMatch[] {
  let best: { branch: CompiledBranch; score: number; params: Readonly<Record<string, string>> } | null = null;
  for (const branch of branches) {
    const result = matchPattern(branch.leafPattern, pathname, true);
    if (!result) continue;
    if (!best || result.score > best.score) {
      best = { branch, score: result.score, params: result.params };
    }
  }
  if (!best) return [];
  return best.branch.chain.map(layer => {
    const layerMatch = matchPattern(layer.pattern, pathname, false);
    return {
      route: layer.route,
      pattern: layer.pattern,
      pathname: layerMatch?.pathname ?? pathname,
      params: best.params,
    };
  });
}

let keyCounter = 0;
function nextKey(): string {
  keyCounter += 1;
  return `loc-${keyCounter}`;
}

function makeLocation(raw: string, state?: unknown, extraSearch?: SearchParamsInput): RouterLocation {
  const split = splitPathAndSearch(raw);
  if (extraSearch) {
    const merged: Record<string, string> = { ...split.searchParams };
    for (const [k, v] of Object.entries(extraSearch)) {
      if (v === undefined || v === null) delete merged[k];
      else merged[k] = String(v);
    }
    const frozen = Object.freeze(merged);
    return Object.freeze({
      key: nextKey(),
      pathname: split.pathname,
      search: serializeSearch(frozen),
      searchParams: frozen,
      state,
    });
  }
  return Object.freeze({
    key: nextKey(),
    pathname: split.pathname,
    search: split.search,
    searchParams: split.searchParams,
    state,
  });
}

/** Create an independent, typed in-memory router with its own history stack and guard registry. */
export function createRouter<const Routes extends readonly RouteDefinition[]>(
  routes: Routes,
  options: CreateRouterOptions<RoutePaths<Routes>> = {},
): Router<RoutePaths<Routes>> {
  type Paths = RoutePaths<Routes>;
  const branches: CompiledBranch[] = [];
  compileBranches(routes, "", [], branches);

  const devChannel = options.devStateKey === false ? null : getDevStateChannel();
  const devKey = typeof options.devStateKey === "string" && options.devStateKey.length > 0
    ? options.devStateKey
    : "__zgui_router";

  let initialEntries: RouterLocation[];
  let clampedStart: number;

  const hydratedRouter = (() => {
    if (!devChannel) return null;
    const raw = devChannel.read(devKey, 1);
    if (raw === null) return null;
    try {
      const parsed = JSON.parse(raw) as {
        entries?: readonly { pathname?: unknown; search?: unknown; state?: unknown }[];
        index?: unknown;
      };
      if (!parsed || !Array.isArray(parsed.entries) || parsed.entries.length === 0) return null;
      const restored: RouterLocation[] = [];
      for (const item of parsed.entries) {
        if (!item || typeof item.pathname !== "string") return null;
        const search = typeof item.search === "string" ? item.search : "";
        restored.push(makeLocation(`${item.pathname}${search}`, item.state));
      }
      const rawIdx = typeof parsed.index === "number" && Number.isInteger(parsed.index)
        ? parsed.index
        : restored.length - 1;
      const idx = Math.max(0, Math.min(restored.length - 1, rawIdx));
      return { entries: restored, index: idx };
    } catch {
      return null;
    }
  })();

  if (hydratedRouter) {
    initialEntries = hydratedRouter.entries;
    clampedStart = hydratedRouter.index;
  } else {
    const rawEntries = options.initialEntries && options.initialEntries.length > 0
      ? options.initialEntries
      : [options.initialPath ?? "/"];
    initialEntries = rawEntries.map((entry, idx) =>
      makeLocation(entry, idx === rawEntries.length - 1 ? options.initialState : undefined),
    );
    clampedStart = options.initialIndex === undefined
      ? initialEntries.length - 1
      : Math.max(0, Math.min(initialEntries.length - 1, Math.trunc(options.initialIndex)));
  }

  const persistRouterState = (list: readonly RouterLocation[], idx: number): void => {
    if (!devChannel) return;
    const serialized = trySerializePlainDevState({
      entries: list.map(entry => ({
        pathname: entry.pathname,
        search: entry.search,
        ...(entry.state !== undefined && trySerializePlainDevState(entry.state) !== null
          ? { state: entry.state }
          : {}),
      })),
      index: idx,
    });
    if (serialized !== null) {
      try {
        devChannel.write(devKey, serialized, 1);
      } catch {
        // Never fail router navigation if dev snapshot budget is exhausted.
      }
    }
  };
  persistRouterState(initialEntries, clampedStart);

  const [entries, setEntries] = createSignal<readonly RouterLocation[]>(initialEntries);
  const [index, setIndex] = createSignal<number>(clampedStart);
  const [isNavigating, setIsNavigating] = createSignal<boolean>(false);

  const guards = new Set<BeforeLeaveGuard>();
  let navSeq = 0;
  let disposed = false;

  const location = createMemo<RouterLocation>(() => {
    const list = entries();
    const idx = Math.max(0, Math.min(list.length - 1, index()));
    return list[idx]!;
  });

  const matches = createMemo<readonly RouteMatch[]>(() => resolveMatches(branches, location().pathname));
  const params = createMemo<Readonly<Record<string, string>>>(() => matches().at(-1)?.params ?? Object.freeze({}));
  const searchParams = createMemo<SearchParams>(() => location().searchParams);
  const canGoBack = createMemo<boolean>(() => index() > 0);
  const canGoForward = createMemo<boolean>(() => index() < entries().length - 1);

  const runTransition = (
    target: RouterLocation,
    delta: number | null,
    replace: boolean,
    commit: () => void,
  ): Promise<boolean> => {
    if (disposed) return Promise.resolve(false);
    const from = untrack(location);
    const seq = ++navSeq;
    const activeGuards = Array.from(guards);

    let prevented = false;
    const event: BeforeLeaveEvent = {
      from,
      to: target,
      delta,
      replace,
      get defaultPrevented() { return prevented; },
      preventDefault() { prevented = true; },
    };

    const finish = (approved: boolean, asyncPath: boolean): boolean => {
      if (disposed || seq !== navSeq) return false;
      if (asyncPath) setIsNavigating(false);
      if (!approved) {
        if (asyncPath) flush();
        return false;
      }
      commit();
      if (asyncPath) flush();
      return true;
    };

    // Execute guards sequentially; stay completely synchronous if no guard returns a Promise.
    for (let i = 0; i < activeGuards.length; i++) {
      const outcome = activeGuards[i]!(event);
      if (outcome && typeof (outcome as Promise<boolean | void>).then === "function") {
        setIsNavigating(true);
        return (async () => {
          try {
            const first = await outcome;
            if (seq !== navSeq || disposed) return false;
            if (first === false || prevented) return finish(false, true);
            for (let j = i + 1; j < activeGuards.length; j++) {
              const next = await activeGuards[j]!(event);
              if (seq !== navSeq || disposed) return false;
              if (next === false || prevented) return finish(false, true);
            }
            return finish(true, true);
          } catch (error) {
            if (seq === navSeq && !disposed) {
              setIsNavigating(false);
              flush();
            }
            throw error;
          }
        })();
      }
      if (outcome === false || prevented) {
        return Promise.resolve(false);
      }
    }

    return Promise.resolve(finish(true, false));
  };

  const navigate = ((to: string, navOptions?: NavigateOptions<string, unknown>): Promise<boolean> => {
    const rawStr = String(to);
    const split = splitPathAndSearch(rawStr);
    const pathname = rawStr.includes(":") || rawStr.includes("*")
      ? buildPath(split.pathname, navOptions?.params)
      : normalizePath(split.pathname);
    const baseQuery = split.searchParams;
    const finalSearch = navOptions?.search !== undefined
      ? normalizeSearchInput({ ...baseQuery, ...navOptions.search })
      : baseQuery;
    const target: RouterLocation = Object.freeze({
      key: nextKey(),
      pathname,
      search: serializeSearch(finalSearch),
      searchParams: finalSearch,
      state: navOptions?.state,
    });
    const replace = navOptions?.replace === true;

    return runTransition(target, null, replace, () => {
      const list = untrack(entries);
      const currentIdx = untrack(index);
      if (replace) {
        const next = list.slice();
        next[currentIdx] = target;
        setEntries(next);
        persistRouterState(next, currentIdx);
      } else {
        const next = [...list.slice(0, currentIdx + 1), target];
        const nextIdx = next.length - 1;
        setEntries(next);
        setIndex(nextIdx);
        persistRouterState(next, nextIdx);
      }
    });
  }) as RouterNavigate<Paths>;

  const go = (delta: number): Promise<boolean> => {
    if (!Number.isFinite(delta) || Math.trunc(delta) === 0) return Promise.resolve(false);
    const step = Math.trunc(delta);
    const list = untrack(entries);
    const currentIdx = untrack(index);
    const nextIdx = currentIdx + step;
    if (nextIdx < 0 || nextIdx >= list.length) return Promise.resolve(false);
    const target = list[nextIdx]!;
    return runTransition(target, step, false, () => {
      setIndex(nextIdx);
      persistRouterState(list, nextIdx);
    });
  };

  const onBeforeLeave = (guard: BeforeLeaveGuard): (() => void) => {
    if (disposed) return () => {};
    guards.add(guard);
    return () => {
      guards.delete(guard);
    };
  };

  const match = <P extends Paths | (string & {})>(pattern: P, exact = true): Accessor<RouteMatch | null> =>
    createMemo<RouteMatch | null>(() => {
      const normPattern = normalizePath(pattern);
      const result = matchPattern(normPattern, location().pathname, exact);
      if (!result) return null;
      return {
        route: { path: normPattern },
        pattern: normPattern,
        pathname: result.pathname,
        params: result.params,
      };
    });

  const dispose = (): void => {
    disposed = true;
    navSeq += 1;
    guards.clear();
  };

  if (getOwner()) onCleanup(dispose);

  return {
    routes,
    entries,
    index,
    location,
    matches,
    params,
    searchParams,
    canGoBack,
    canGoForward,
    isNavigating,
    navigate,
    back: () => go(-1),
    forward: () => go(1),
    go,
    match,
    onBeforeLeave,
    dispose,
  };
}

/** Structural subset of `@hewel/zgui-services` control message payloads. */
export interface ControlMessageLike {
  readonly command: string;
  readonly payload?: unknown;
}

/** Control message subscriber compatible with `onControlMessage` from `@hewel/zgui-services`. */
export type ControlMessageSubscriber = (
  handler: (message: ControlMessageLike) => void,
) => () => void;

/** Parsed navigation request derived from a host control message. */
export interface ControlNavigationTarget {
  readonly to: string;
  readonly params?: Readonly<Record<string, string>> | undefined;
  readonly search?: SearchParamsInput | undefined;
  readonly replace?: boolean | undefined;
  readonly state?: unknown;
}

/** Options for `bindControlNavigation(router, options)`. */
export interface ControlNavigationOptions {
  readonly subscribe: ControlMessageSubscriber;
  readonly command?: string | undefined;
  readonly parse?: ((payload: unknown) => ControlNavigationTarget | null) | undefined;
}

function defaultParseControlPayload(payload: unknown): ControlNavigationTarget | null {
  if (typeof payload === "string" && payload.trim().length > 0) {
    return { to: payload };
  }
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return null;
  const record = payload as Record<string, unknown>;
  const target = typeof record.to === "string"
    ? record.to
    : typeof record.path === "string"
      ? record.path
      : null;
  if (!target) return null;
  const replace = typeof record.replace === "boolean" ? record.replace : undefined;
  const search = record.search && typeof record.search === "object" && !Array.isArray(record.search)
    ? (record.search as SearchParamsInput)
    : undefined;
  const params = record.params && typeof record.params === "object" && !Array.isArray(record.params)
    ? (record.params as Readonly<Record<string, string>>)
    : undefined;
  return { to: target, params, search, replace, state: record.state };
}

/** Bind host control messages (e.g. `onControlMessage` / `native-host msg`) to a router instance. */
export function bindControlNavigation<Paths extends string>(
  router: Router<Paths>,
  options: ControlNavigationOptions,
): () => void {
  const command = options.command ?? "navigate";
  const parse = options.parse ?? defaultParseControlPayload;
  const unsubscribe = options.subscribe(message => {
    if (message.command !== command) return;
    const parsed = parse(message.payload);
    if (!parsed) return;
    const nav = router.navigate as (to: string, opts?: NavigateOptions) => Promise<boolean>;
    void nav(parsed.to, {
      params: parsed.params,
      search: parsed.search,
      replace: parsed.replace,
      state: parsed.state,
    });
  });
  if (getOwner()) onCleanup(unsubscribe);
  return unsubscribe;
}
