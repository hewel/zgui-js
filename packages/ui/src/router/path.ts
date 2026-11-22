/** Parsed query string parameters (last value wins for duplicate keys). */
export type SearchParams = Readonly<Record<string, string>>;

/** Input map accepted when building or updating query parameters. */
export type SearchParamsInput = Readonly<
  Record<string, string | number | boolean | null | undefined>
>;

type TrimLeadingSlash<S extends string> = S extends `/${infer Rest}` ? TrimLeadingSlash<Rest> : S;
type TrimTrailingSlash<S extends string> = S extends "/"
  ? "/"
  : S extends `${infer Head}/`
    ? TrimTrailingSlash<Head>
    : S;

/** Normalize a route pattern string at the type level. */
export type NormalizePath<P extends string> =
  TrimLeadingSlash<P> extends "" ? "/" : TrimTrailingSlash<`/${TrimLeadingSlash<P>}`>;

/** Join a parent route prefix and a child route segment at the type level. */
export type JoinPaths<Parent extends string, Child extends string> = Child extends ""
  ? Parent extends ""
    ? "/"
    : NormalizePath<Parent>
  : Child extends `/${string}`
    ? NormalizePath<Child>
    : Parent extends "" | "/"
      ? NormalizePath<Child>
      : NormalizePath<`${TrimTrailingSlash<Parent>}/${TrimLeadingSlash<Child>}`>;

type SegmentParam<Segment extends string> = Segment extends `:${infer Name}`
  ? { readonly [K in Name]: string }
  : Segment extends "*"
    ? { readonly "*": string }
    : Segment extends `*${infer Name}`
      ? { readonly [K in Name]: string }
      : {};

type MergeIntersection<T> = { readonly [K in keyof T]: T[K] };

type ExtractPathParamsInner<P extends string> = P extends `${infer Head}/${infer Tail}`
  ? SegmentParam<Head> & ExtractPathParamsInner<Tail>
  : SegmentParam<P>;

/** Extract typed `:param` and `*` splat fields from a route pattern. */
export type ExtractRouteParams<P extends string> = keyof ExtractPathParamsInner<
  TrimLeadingSlash<P>
> extends never
  ? Readonly<Record<string, never>>
  : MergeIntersection<ExtractPathParamsInner<TrimLeadingSlash<P>>>;

/** Whether a route pattern requires parameter values to materialize a concrete path. */
export type HasRouteParams<P extends string> = keyof ExtractPathParamsInner<
  TrimLeadingSlash<P>
> extends never
  ? false
  : true;

type ConcreteSegment<Segment extends string> = Segment extends `:${string}` | `*${string}`
  ? string
  : Segment;

type ConcreteInner<P extends string> = P extends `${infer Head}/${infer Tail}`
  ? `${ConcreteSegment<Head>}/${ConcreteInner<Tail>}`
  : ConcreteSegment<P>;

/** Concrete pathname shape matching a route pattern (e.g. `"/docs/:id"` -> `` `/docs/${string}` ``). */
export type ConcretePath<P extends string> =
  NormalizePath<P> extends "/" ? "/" : `/${ConcreteInner<TrimLeadingSlash<NormalizePath<P>>>}`;

/** Normalize a runtime pathname: always starts with `/`, collapses slashes, trims trailing `/`. */
export function normalizePath(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed || trimmed === "/") return "/";
  const stripped = trimmed.split("#", 1)[0]!.split("?", 1)[0]!;
  const parts = stripped.split("/").filter(part => part.length > 0);
  return parts.length === 0 ? "/" : `/${parts.join("/")}`;
}

/** Join parent and child route patterns into a canonical normalized pattern. */
export function joinPaths(parent: string, child: string): string {
  const cleanChild = child.trim();
  if (!cleanChild) return normalizePath(parent || "/");
  if (cleanChild.startsWith("/")) return normalizePath(cleanChild);
  const base = normalizePath(parent || "/");
  return base === "/" ? normalizePath(`/${cleanChild}`) : normalizePath(`${base}/${cleanChild}`);
}

/** Split a path string into normalized `pathname` and canonical `search` (`""` or `"?a=1"`). */
export function splitPathAndSearch(raw: string): {
  pathname: string;
  search: string;
  searchParams: SearchParams;
} {
  const noHash = raw.split("#", 1)[0] ?? "";
  const queryIdx = noHash.indexOf("?");
  const rawPath = queryIdx >= 0 ? noHash.slice(0, queryIdx) : noHash;
  const rawQuery = queryIdx >= 0 ? noHash.slice(queryIdx + 1) : "";
  const pathname = normalizePath(rawPath || "/");
  const searchParams = parseSearch(rawQuery);
  const search = serializeSearch(searchParams);
  return { pathname, search, searchParams };
}

function decodePart(value: string): string {
  try {
    return decodeURIComponent(value.replace(/\+/g, " "));
  } catch {
    return value;
  }
}

/** Parse a query string (with or without leading `?`) into an immutable key/value map. */
export function parseSearch(raw: string): SearchParams {
  const query = raw.startsWith("?") ? raw.slice(1) : raw;
  const out: Record<string, string> = {};
  if (!query) return Object.freeze(out);
  for (const entry of query.split("&")) {
    if (!entry) continue;
    const eq = entry.indexOf("=");
    if (eq < 0) {
      const key = decodePart(entry);
      if (key) out[key] = "";
    } else {
      const key = decodePart(entry.slice(0, eq));
      if (key) out[key] = decodePart(entry.slice(eq + 1));
    }
  }
  return Object.freeze(out);
}

/** Serialize query parameters into `""` or `"?k=v&..."` without browser globals. */
export function serializeSearch(params?: SearchParamsInput | null): string {
  if (!params) return "";
  const pairs: string[] = [];
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue;
    pairs.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
  }
  return pairs.length > 0 ? `?${pairs.join("&")}` : "";
}

/** Merge normalized search input into a frozen `SearchParams` record. */
export function normalizeSearchInput(params?: SearchParamsInput | null): SearchParams {
  const out: Record<string, string> = {};
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      if (value === undefined || value === null) continue;
      out[key] = String(value);
    }
  }
  return Object.freeze(out);
}

/** Interpolate `:param` and `*` splat placeholders in a pattern to produce a concrete pathname. */
export function buildPath(
  pattern: string,
  params?: Readonly<Record<string, string>> | null,
): string {
  const normalized = normalizePath(pattern);
  if (normalized === "/") return "/";
  const segments = normalized.slice(1).split("/");
  const built: string[] = [];
  for (const segment of segments) {
    if (segment.startsWith(":")) {
      const name = segment.slice(1);
      const value = params?.[name];
      if (value === undefined) {
        throw new TypeError(`Missing route parameter ":${name}" for pattern "${pattern}"`);
      }
      built.push(encodeURIComponent(value));
    } else if (segment.startsWith("*")) {
      const name = segment.length > 1 ? segment.slice(1) : "*";
      const value = params?.[name] ?? "";
      if (value) {
        for (const piece of value.split("/").filter(Boolean)) {
          built.push(encodeURIComponent(piece));
        }
      }
    } else {
      built.push(segment);
    }
  }
  return built.length === 0 ? "/" : `/${built.join("/")}`;
}

export interface PatternMatchResult {
  readonly params: Readonly<Record<string, string>>;
  readonly score: number;
  readonly pathname: string;
}

/** Match a normalized pathname against a normalized pattern. */
export function matchPattern(
  pattern: string,
  pathname: string,
  exact = true,
): PatternMatchResult | null {
  const normPattern = normalizePath(pattern);
  const normPath = normalizePath(pathname);
  const patSegs = normPattern === "/" ? [] : normPattern.slice(1).split("/");
  const pathSegs = normPath === "/" ? [] : normPath.slice(1).split("/");
  const params: Record<string, string> = {};
  let score = 0;

  let pIdx = 0;
  for (; pIdx < patSegs.length; pIdx++) {
    const seg = patSegs[pIdx]!;
    if (seg.startsWith("*")) {
      const name = seg.length > 1 ? seg.slice(1) : "*";
      params[name] = pathSegs.slice(pIdx).map(decodePart).join("/");
      score += 1;
      return { params: Object.freeze(params), score, pathname: normPath };
    }
    if (pIdx >= pathSegs.length) return null;
    const actual = pathSegs[pIdx]!;
    if (seg.startsWith(":")) {
      params[seg.slice(1)] = decodePart(actual);
      score += 5;
    } else if (seg === actual) {
      score += 10;
    } else {
      return null;
    }
  }

  if (exact && pIdx !== pathSegs.length) return null;
  // Non-wildcard exact matches always outrank wildcard splats (including "/" vs "/*").
  score += 1000;
  const matchedPath = pIdx === 0 ? "/" : `/${pathSegs.slice(0, pIdx).join("/")}`;
  return { params: Object.freeze(params), score, pathname: matchedPath };
}
