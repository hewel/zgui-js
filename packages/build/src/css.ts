import { createRequire } from "node:module";
import { readdir, readFile, stat, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { Features, transform, type MediaCondition, type SelectorComponent } from "lightningcss";
import { canonicalJson, sha256 } from "./identity.ts";

const require = createRequire(import.meta.url);
const lightningcssPkg = require("lightningcss/package.json") as { version: string };

export const CSS_MODULES_PATTERN = "[hash]_[local]";

export interface StyleSystem {
  name: "lightningcss";
  profile: "lightningcss-v1";
  configDigest: string;
}

export interface TransformedCss {
  css: string;
  exports: Record<string, string>;
}

const STYLE_CONFIG = {
  name: "lightningcss",
  profile: "lightningcss-v1",
  lightningcssVersion: lightningcssPkg.version,
  cssModules: {
    pattern: CSS_MODULES_PATTERN,
    dashedIdents: false,
  },
  nesting: true,
  targets: null,
} as const;

const STYLE_CONFIG_DIGEST = sha256(canonicalJson(STYLE_CONFIG));

export function styleSystem(): StyleSystem {
  return {
    name: "lightningcss",
    profile: "lightningcss-v1",
    configDigest: STYLE_CONFIG_DIGEST,
  };
}

function hasPrefersReducedMotion(condition: MediaCondition | null | undefined): boolean {
  if (!condition) return false;
  switch (condition.type) {
    case "feature": {
      const feat = condition.value;
      if (feat.type === "plain" || feat.type === "range" || feat.type === "boolean") {
        return feat.name.toLowerCase() === "prefers-reduced-motion";
      }
      if (feat.type === "interval") {
        return feat.name.toLowerCase() === "prefers-reduced-motion";
      }
      return false;
    }
    case "not":
      return hasPrefersReducedMotion(condition.value);
    case "operation":
      return condition.conditions.some(item => hasPrefersReducedMotion(item));
    default:
      return false;
  }
}

function selectorContainsDirAttr(components: readonly SelectorComponent[]): boolean {
  for (const part of components) {
    if (part.type === "attribute" && part.name.toLowerCase() === "dir") return true;
    if (part.type === "pseudo-class" && "selectors" in part && Array.isArray(part.selectors)) {
      for (const nested of part.selectors) {
        if (Array.isArray(nested) && selectorContainsDirAttr(nested as SelectorComponent[])) {
          return true;
        }
      }
    }
  }
  return false;
}

function checkSelector(
  components: readonly SelectorComponent[],
  filename: string,
  cssModules: boolean,
): void {
  let hasHtmlType = false;
  let hasDirAttr = false;

  const flushCompound = (): void => {
    if (hasHtmlType && hasDirAttr) {
      throw new Error(`Unsupported native CSS feature in ${filename}: html[dir]`);
    }
    hasHtmlType = false;
    hasDirAttr = false;
  };

  for (const part of components) {
    if (part.type === "combinator") {
      flushCompound();
      continue;
    }
    if (part.type === "type" && part.name.toLowerCase() === "html") {
      hasHtmlType = true;
    }
    if (part.type === "attribute" && part.name.toLowerCase() === "dir") {
      hasDirAttr = true;
    }
    if (cssModules && part.type === "pseudo-class") {
      if (part.kind === "custom" && part.name === "export") {
        throw new Error(`Unsupported CSS Modules ICSS :export rule in ${filename}`);
      }
      if (part.kind === "custom-function" && part.name === "import") {
        throw new Error(`Unsupported CSS Modules ICSS :import rule in ${filename}`);
      }
    }
    if (part.type === "pseudo-class" && "selectors" in part && Array.isArray(part.selectors)) {
      for (const nested of part.selectors) {
        if (Array.isArray(nested)) {
          const list = nested as SelectorComponent[];
          if (selectorContainsDirAttr(list)) hasDirAttr = true;
          checkSelector(list, filename, cssModules);
        }
      }
    }
  }
  flushCompound();
}

/**
 * Validates and transforms native CSS or a locally scoped `*.module.css` file via Lightning CSS.
 * Runs with no Browserslist targets so Stylo keeps native tokens (`oklch`, `color-mix`, `--zui-*`),
 * flattens CSS nesting (`&`), and rejects unsupported Stylo/ICSS features at the AST level.
 */
export function transformStylesheet(
  css: string,
  {
    filename,
    cssModules = false,
  }: {
    filename: string;
    cssModules?: boolean;
  },
): TransformedCss {
  let result;
  try {
    result = transform({
      filename,
      code: Buffer.from(css, "utf8"),
      include: Features.Nesting,
      cssModules: cssModules
        ? {
            pattern: CSS_MODULES_PATTERN,
            dashedIdents: false,
          }
        : false,
      visitor: {
        Rule(rule) {
          switch (rule.type) {
            case "import":
              throw new Error(`Unsupported native CSS feature in ${filename}: @import`);
            case "font-face":
              throw new Error(`Unsupported native CSS feature in ${filename}: @font-face`);
            case "container":
              throw new Error(`Unsupported native CSS feature in ${filename}: @container`);
            case "property":
              throw new Error(`Unsupported native CSS feature in ${filename}: @property`);
            case "unknown":
              if (rule.value.name.toLowerCase() === "value") {
                throw new Error(`Unsupported CSS Modules @value rule in ${filename}`);
              }
              break;
            default:
              break;
          }
        },
        Url(u) {
          throw new Error(`Unsupported native CSS feature in ${filename}: url(${u.url})`);
        },
        Image(img) {
          if (img.type === "image-set") {
            throw new Error(`Unsupported native CSS feature in ${filename}: image-set()`);
          }
        },
        Function(fn) {
          const lower = fn.name.toLowerCase();
          if (lower === "image-set" || lower === "-webkit-image-set") {
            throw new Error(`Unsupported native CSS feature in ${filename}: ${fn.name}()`);
          }
        },
        MediaQuery(mq) {
          if (hasPrefersReducedMotion(mq.condition)) {
            throw new Error(
              `Unsupported native CSS feature in ${filename}: prefers-reduced-motion`,
            );
          }
        },
        Selector(sel) {
          checkSelector(sel, filename, cssModules);
        },
      },
    });
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("Unsupported ")) {
      throw error;
    }
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`CSS compilation failed for ${filename}: ${message}`);
  }

  const exportsMap: Record<string, string> = {};
  if (cssModules && result.exports) {
    for (const key of Object.keys(result.exports).sort()) {
      const entry = result.exports[key];
      if (!entry) continue;
      if (entry.composes.length > 0) {
        throw new Error(
          `Unsupported CSS Modules feature in ${filename}: 'composes' on '.${key}' is not supported in v1`,
        );
      }
      exportsMap[key] = entry.name;
    }
  }

  return {
    css: Buffer.from(result.code).toString("utf8").trim(),
    exports: exportsMap,
  };
}

export function validateStylesheet(css: string, filename = "app.css"): string {
  return transformStylesheet(css, { filename, cssModules: false }).css;
}

const VALID_TS_PROP = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const GLOB_CHAR = /[*?[{]/;
const IGNORED_SCAN_DIRS = new Set([
  "node_modules",
  "dist",
  "dist-package",
  "target",
  ".git",
  ".zgui-generations",
  ".direnv",
  ".scratch",
  ".cache",
]);

export interface SyncCssModuleTypesOptions {
  project?: string | undefined;
  appDirectory?: string | undefined;
  include?: readonly string[] | undefined;
  cwd?: string | undefined;
}

interface ScanTarget {
  dir: string;
  recursive: boolean;
}

interface TsconfigSpec {
  include?: { patterns: string[]; dir: string };
  files?: { entries: string[]; dir: string };
  exclude?: { patterns: string[]; dir: string };
}

function isIgnoredDirName(name: string): boolean {
  return (
    IGNORED_SCAN_DIRS.has(name) ||
    name.startsWith(".dist-") ||
    name.startsWith(".shell-compose-stage-")
  );
}

function toPosix(p: string): string {
  return p.split(path.sep).join("/");
}

function stripTrailingSlash(p: string): string {
  if (p.length <= 1) return p;
  return p.replace(/\/+$/, "") || "/";
}

function isSameOrInside(candidate: string, ancestor: string): boolean {
  const rel = path.relative(ancestor, candidate);
  return rel === "" || (!rel.startsWith(`..${path.sep}`) && rel !== ".." && !path.isAbsolute(rel));
}

async function safeStat(targetPath: string) {
  try {
    return await stat(targetPath);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT" || code === "ENOTDIR") return undefined;
    throw error;
  }
}

function splitGlobBase(
  baseDir: string,
  posixPattern: string,
): { staticBase: string; globRest: string } {
  const segments = posixPattern.split("/");
  const firstGlob = segments.findIndex(seg => GLOB_CHAR.test(seg));
  if (firstGlob === -1) {
    return { staticBase: path.resolve(baseDir, posixPattern), globRest: "" };
  }
  const prefix = segments.slice(0, firstGlob).join("/");
  const staticBase =
    firstGlob === 0
      ? path.resolve(baseDir)
      : path.resolve(baseDir, prefix === "" && posixPattern.startsWith("/") ? "/" : prefix);
  return { staticBase, globRest: segments.slice(firstGlob).join("/") };
}

function expandConfigDir(pattern: string, leafConfigDir: string): string {
  return pattern.replaceAll("${configDir}", toPosix(leafConfigDir));
}

async function resolveTsconfigExtendsPath(
  specifier: string,
  fromConfigPath: string,
  leafConfigDir: string,
): Promise<string> {
  const expanded = expandConfigDir(specifier, leafConfigDir);
  if (
    expanded.startsWith("./") ||
    expanded.startsWith("../") ||
    expanded === "." ||
    expanded === ".." ||
    path.isAbsolute(expanded)
  ) {
    const base = path.resolve(path.dirname(fromConfigPath), expanded);
    for (const candidate of [base, `${base}.json`, path.join(base, "tsconfig.json")]) {
      const info = await safeStat(candidate);
      if (info?.isFile()) return candidate;
    }
    throw new Error(`Cannot resolve tsconfig extends "${specifier}" from ${fromConfigPath}`);
  }
  const req = createRequire(fromConfigPath);
  for (const candidate of [expanded, `${expanded}.json`, `${expanded}/tsconfig.json`]) {
    try {
      const resolved = req.resolve(candidate);
      const info = await safeStat(resolved);
      if (info?.isFile()) return resolved;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code !== "MODULE_NOT_FOUND" && code !== "ERR_PACKAGE_PATH_NOT_EXPORTED") throw error;
    }
  }
  throw new Error(`Cannot resolve tsconfig extends "${specifier}" from ${fromConfigPath}`);
}

function stringList(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.filter((item): item is string => typeof item === "string");
}

async function collectTsconfigSpec(
  configPath: string,
  leafConfigDir: string,
  visited: Set<string>,
): Promise<TsconfigSpec> {
  const resolved = path.resolve(configPath);
  if (visited.has(resolved)) return {};
  visited.add(resolved);

  const rawText = await readFile(resolved, "utf8");
  const parsed = Bun.JSONC.parse(rawText) as Record<string, unknown> | null;
  if (!parsed || typeof parsed !== "object") return {};

  const configDir = path.dirname(resolved);
  const ownInclude = stringList(parsed.include);
  const ownFiles = stringList(parsed.files);
  const ownExclude = stringList(parsed.exclude);
  const hasOwnInclusion = ownInclude !== undefined || ownFiles !== undefined;

  const spec: TsconfigSpec = {
    ...(ownInclude !== undefined ? { include: { patterns: ownInclude, dir: configDir } } : {}),
    ...(ownFiles !== undefined ? { files: { entries: ownFiles, dir: configDir } } : {}),
    ...(ownExclude !== undefined ? { exclude: { patterns: ownExclude, dir: configDir } } : {}),
  };

  if (hasOwnInclusion && spec.exclude !== undefined) {
    return spec;
  }

  const extendsList =
    typeof parsed.extends === "string" ? [parsed.extends] : (stringList(parsed.extends) ?? []);

  let inheritedInclude = hasOwnInclusion ? spec.include : undefined;
  let inheritedFiles = hasOwnInclusion ? spec.files : undefined;
  let inheritedExclude = spec.exclude;

  for (let i = extendsList.length - 1; i >= 0; i--) {
    if (
      (inheritedInclude !== undefined || inheritedFiles !== undefined) &&
      inheritedExclude !== undefined
    ) {
      break;
    }
    const basePath = await resolveTsconfigExtendsPath(extendsList[i]!, resolved, leafConfigDir);
    const parentSpec = await collectTsconfigSpec(basePath, leafConfigDir, visited);
    if (inheritedInclude === undefined && inheritedFiles === undefined) {
      inheritedInclude = parentSpec.include;
      inheritedFiles = parentSpec.files;
    }
    if (inheritedExclude === undefined) {
      inheritedExclude = parentSpec.exclude;
    }
  }

  return {
    ...(inheritedInclude !== undefined ? { include: inheritedInclude } : {}),
    ...(inheritedFiles !== undefined ? { files: inheritedFiles } : {}),
    ...(inheritedExclude !== undefined ? { exclude: inheritedExclude } : {}),
  };
}

async function expandGlobDirectories(baseDir: string, posixDirPattern: string): Promise<string[]> {
  if (!GLOB_CHAR.test(posixDirPattern)) {
    const target = path.resolve(baseDir, posixDirPattern);
    const info = await safeStat(target);
    return info?.isDirectory() ? [target] : [];
  }
  const { staticBase, globRest } = splitGlobBase(baseDir, posixDirPattern);
  const staticInfo = await safeStat(staticBase);
  if (!staticInfo?.isDirectory()) return [];
  const dirs: string[] = [];
  const glob = new Bun.Glob(globRest);
  for await (const match of glob.scan({
    cwd: staticBase,
    onlyFiles: false,
    followSymlinks: true,
  })) {
    const candidate = path.resolve(staticBase, match);
    const info = await safeStat(candidate);
    if (info?.isDirectory()) dirs.push(candidate);
  }
  return dirs;
}

async function resolveTsconfigCssRoots(
  projectPath: string,
  targets: Map<string, boolean>,
  directCssFiles: Set<string>,
  directDtsFiles: Set<string>,
  excludedDirs: Set<string>,
): Promise<void> {
  const resolvedProject = path.resolve(projectPath);
  const leafConfigDir = path.dirname(resolvedProject);
  const spec = await collectTsconfigSpec(resolvedProject, leafConfigDir, new Set());

  const addTarget = (dir: string, recursive: boolean) => {
    const abs = path.resolve(dir);
    targets.set(abs, (targets.get(abs) ?? false) || recursive);
  };

  if (spec.exclude) {
    for (const rawPattern of spec.exclude.patterns) {
      const normalized = stripTrailingSlash(
        toPosix(expandConfigDir(rawPattern, leafConfigDir)).replace(/\/+\*\*?(?:\/\*+)?$/, ""),
      );
      if (!normalized || GLOB_CHAR.test(normalized)) continue;
      excludedDirs.add(path.resolve(spec.exclude.dir, normalized));
    }
  }

  if (spec.files) {
    for (const rawEntry of spec.files.entries) {
      const expanded = expandConfigDir(rawEntry, leafConfigDir);
      const target = path.resolve(spec.files.dir, expanded);
      const info = await safeStat(target);
      if (!info) continue;
      if (info.isDirectory()) {
        addTarget(target, true);
      } else if (info.isFile()) {
        if (target.endsWith(".module.css")) {
          directCssFiles.add(target);
        } else {
          const parent = path.dirname(target);
          addTarget(parent, parent !== leafConfigDir);
        }
      }
    }
  }

  const includePatterns =
    spec.include !== undefined
      ? spec.include
      : spec.files !== undefined
        ? undefined
        : { patterns: ["**/*"], dir: leafConfigDir };

  if (includePatterns) {
    for (const rawPattern of includePatterns.patterns) {
      const normalized = stripTrailingSlash(toPosix(expandConfigDir(rawPattern, leafConfigDir)));
      if (!normalized) continue;

      if (!GLOB_CHAR.test(normalized)) {
        const target = path.resolve(includePatterns.dir, normalized);
        const info = await safeStat(target);
        if (!info) continue;
        if (info.isDirectory()) {
          addTarget(target, true);
        } else if (info.isFile()) {
          if (target.endsWith(".module.css")) {
            directCssFiles.add(target);
          } else {
            const parent = path.dirname(target);
            addTarget(parent, parent !== leafConfigDir);
          }
        }
        continue;
      }

      if (normalized.includes("**")) {
        const segments = normalized.split("/");
        const ddIndex = segments.findIndex(seg => seg.includes("**"));
        const prefix =
          ddIndex <= 0
            ? "."
            : segments.slice(0, ddIndex).join("/") || (normalized.startsWith("/") ? "/" : ".");
        for (const dir of await expandGlobDirectories(includePatterns.dir, prefix)) {
          addTarget(dir, true);
        }
        continue;
      }

      if (normalized.endsWith(".module.css")) {
        const { staticBase, globRest } = splitGlobBase(includePatterns.dir, normalized);
        if ((await safeStat(staticBase))?.isDirectory()) {
          for await (const match of new Bun.Glob(globRest).scan({
            cwd: staticBase,
            onlyFiles: true,
            followSymlinks: true,
          })) {
            directCssFiles.add(path.resolve(staticBase, match));
          }
          for await (const match of new Bun.Glob(`${globRest}.d.ts`).scan({
            cwd: staticBase,
            onlyFiles: true,
            followSymlinks: true,
          })) {
            directDtsFiles.add(path.resolve(staticBase, match));
          }
        }
        continue;
      }

      const dirPattern = path.posix.dirname(normalized);
      for (const dir of await expandGlobDirectories(includePatterns.dir, dirPattern)) {
        addTarget(dir, path.resolve(dir) !== leafConfigDir);
      }
    }
  }
}

async function resolveExplicitIncludes(
  include: readonly string[],
  baseCwd: string,
  targets: Map<string, boolean>,
  directCssFiles: Set<string>,
  directDtsFiles: Set<string>,
): Promise<void> {
  const addTarget = (dir: string, recursive: boolean) => {
    const abs = path.resolve(dir);
    targets.set(abs, (targets.get(abs) ?? false) || recursive);
  };

  for (const rawEntry of include) {
    const normalized = stripTrailingSlash(toPosix(rawEntry));
    if (!normalized) continue;
    if (!GLOB_CHAR.test(normalized)) {
      const target = path.resolve(baseCwd, normalized);
      const info = await safeStat(target);
      if (!info) continue;
      if (info.isDirectory()) {
        addTarget(target, true);
      } else if (info.isFile() && target.endsWith(".module.css")) {
        directCssFiles.add(target);
      }
      continue;
    }

    const { staticBase, globRest } = splitGlobBase(baseCwd, normalized);
    if (!(await safeStat(staticBase))?.isDirectory()) continue;
    for await (const match of new Bun.Glob(globRest).scan({
      cwd: staticBase,
      onlyFiles: false,
      followSymlinks: true,
    })) {
      const candidate = path.resolve(staticBase, match);
      const info = await safeStat(candidate);
      if (info?.isDirectory()) {
        addTarget(candidate, true);
      } else if (info?.isFile() && candidate.endsWith(".module.css")) {
        directCssFiles.add(candidate);
      }
    }
    if (normalized.endsWith(".module.css")) {
      for await (const match of new Bun.Glob(`${globRest}.d.ts`).scan({
        cwd: staticBase,
        onlyFiles: true,
        followSymlinks: true,
      })) {
        directDtsFiles.add(path.resolve(staticBase, match));
      }
    }
  }
}

export function formatCssModuleDts(exportsMap: Readonly<Record<string, string>>): string {
  const keys = Object.keys(exportsMap).sort();
  if (keys.length === 0) {
    return [
      "// Generated by @hewel/zgui-build. Do not edit manually.",
      "declare const styles: Readonly<Record<never, never>>;",
      "export default styles;",
      "",
    ].join("\n");
  }
  const lines = [
    "// Generated by @hewel/zgui-build. Do not edit manually.",
    "declare const styles: {",
    ...keys.map(key => {
      const prop = VALID_TS_PROP.test(key) ? key : JSON.stringify(key);
      return `  readonly ${prop}: string;`;
    }),
    "};",
    "export default styles;",
    "",
  ];
  return lines.join("\n");
}

async function scanCssModules(
  directory: string,
  modules: Set<string>,
  declarations: Set<string>,
  {
    recursive = true,
    excludedDirs,
  }: { recursive?: boolean; excludedDirs?: ReadonlySet<string> } = {},
): Promise<void> {
  if (excludedDirs?.has(path.resolve(directory))) return;
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT" || code === "ENOTDIR") return;
    throw error;
  }
  entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const entry of entries) {
    if (isIgnoredDirName(entry.name)) {
      continue;
    }
    const full = path.resolve(directory, entry.name);
    if (entry.isDirectory()) {
      if (recursive) {
        await scanCssModules(full, modules, declarations, { recursive: true, excludedDirs });
      }
    } else if (entry.isFile()) {
      if (entry.name.endsWith(".module.css")) {
        modules.add(full);
      } else if (entry.name.endsWith(".module.css.d.ts")) {
        declarations.add(full);
      }
    }
  }
}

async function resolveCssScanInputs(options: SyncCssModuleTypesOptions = {}): Promise<{
  baseCwd: string;
  resolvedAppDir?: string;
  resolvedProject?: string;
  targets: ScanTarget[];
  directCssFiles: Set<string>;
  directDtsFiles: Set<string>;
  excludedDirs: Set<string>;
}> {
  const baseCwd = path.resolve(options.cwd ?? process.cwd());
  const resolvedAppDir =
    options.appDirectory !== undefined ? path.resolve(baseCwd, options.appDirectory) : undefined;
  const targets = new Map<string, boolean>();
  const directCssFiles = new Set<string>();
  const directDtsFiles = new Set<string>();
  const excludedDirs = new Set<string>();

  if (resolvedAppDir !== undefined) {
    targets.set(path.resolve(resolvedAppDir, "src"), true);
  }

  let resolvedProject: string | undefined;
  if (options.project !== undefined) {
    resolvedProject = path.resolve(baseCwd, options.project);
    const info = await safeStat(resolvedProject);
    if (!info?.isFile()) {
      throw new Error(`tsconfig.json not found: ${resolvedProject}`);
    }
  } else if (resolvedAppDir !== undefined) {
    const candidate = path.join(resolvedAppDir, "tsconfig.json");
    if ((await safeStat(candidate))?.isFile()) {
      resolvedProject = candidate;
    }
  } else if (options.include === undefined) {
    const candidate = path.join(baseCwd, "tsconfig.json");
    if ((await safeStat(candidate))?.isFile()) {
      resolvedProject = candidate;
    }
    const cwdSrc = path.join(baseCwd, "src");
    if ((await safeStat(cwdSrc))?.isDirectory()) {
      targets.set(cwdSrc, true);
    }
  }

  if (resolvedProject !== undefined) {
    await resolveTsconfigCssRoots(
      resolvedProject,
      targets,
      directCssFiles,
      directDtsFiles,
      excludedDirs,
    );
  }

  if (options.include !== undefined && options.include.length > 0) {
    await resolveExplicitIncludes(
      options.include,
      baseCwd,
      targets,
      directCssFiles,
      directDtsFiles,
    );
  }

  const orderedTargets: ScanTarget[] = [...targets.entries()]
    .map(([dir, recursive]) => ({ dir, recursive }))
    .sort((a, b) => (a.dir < b.dir ? -1 : a.dir > b.dir ? 1 : 0));

  return {
    baseCwd,
    ...(resolvedAppDir !== undefined ? { resolvedAppDir } : {}),
    ...(resolvedProject !== undefined ? { resolvedProject } : {}),
    targets: orderedTargets,
    directCssFiles,
    directDtsFiles,
    excludedDirs,
  };
}

export async function resolveCssModuleWatchDirectories(
  options: SyncCssModuleTypesOptions = {},
): Promise<string[]> {
  const { targets, directCssFiles, excludedDirs } = await resolveCssScanInputs(options);
  const candidates = new Set<string>();
  for (const target of targets) {
    if (excludedDirs.has(target.dir)) continue;
    if ((await safeStat(target.dir))?.isDirectory()) {
      candidates.add(target.dir);
    }
  }
  for (const file of directCssFiles) {
    const dir = path.dirname(file);
    if (!excludedDirs.has(dir) && (await safeStat(dir))?.isDirectory()) {
      candidates.add(dir);
    }
  }

  const sorted = [...candidates].sort(
    (a, b) => a.length - b.length || (a < b ? -1 : a > b ? 1 : 0),
  );
  const kept: string[] = [];
  for (const dir of sorted) {
    if (kept.some(ancestor => isSameOrInside(dir, ancestor))) continue;
    kept.push(dir);
  }
  return kept.sort();
}

function formatDiagnosticFilename(
  cssPath: string,
  diagnosticRoot: string,
  baseCwd: string,
): string {
  const fromRoot = path.relative(diagnosticRoot, cssPath);
  if (
    fromRoot &&
    !fromRoot.startsWith(`..${path.sep}`) &&
    fromRoot !== ".." &&
    !path.isAbsolute(fromRoot)
  ) {
    return toPosix(fromRoot);
  }
  const fromCwd = path.relative(baseCwd, cssPath);
  if (
    fromCwd &&
    !fromCwd.startsWith(`..${path.sep}`) &&
    fromCwd !== ".." &&
    !path.isAbsolute(fromCwd)
  ) {
    return toPosix(fromCwd);
  }
  return fromRoot ? toPosix(fromRoot) : path.basename(cssPath);
}

/**
 * Generates exact `*.module.css.d.ts` declarations for every `*.module.css` file under the
 * configured app, tsconfig, and explicit include roots, skipping writes when unchanged
 * (watch-safe) and deleting stale declarations on rename/delete.
 */
export async function syncCssModuleTypes(
  options: SyncCssModuleTypesOptions = {},
): Promise<string[]> {
  const {
    baseCwd,
    resolvedAppDir,
    resolvedProject,
    targets,
    directCssFiles,
    directDtsFiles,
    excludedDirs,
  } = await resolveCssScanInputs(options);

  const moduleSet = new Set<string>(directCssFiles);
  const existingDtsSet = new Set<string>(directDtsFiles);

  for (const target of targets) {
    await scanCssModules(target.dir, moduleSet, existingDtsSet, {
      recursive: target.recursive,
      excludedDirs,
    });
  }

  const modules = [...moduleSet].sort();
  const existingDts = [...existingDtsSet].sort();
  const diagnosticRoot =
    resolvedAppDir ?? (resolvedProject ? path.dirname(resolvedProject) : baseCwd);

  const expectedDts = new Set<string>();
  for (const cssPath of modules) {
    const source = await readFile(cssPath, "utf8");
    const diagnosticName = formatDiagnosticFilename(cssPath, diagnosticRoot, baseCwd);
    const { exports: exportsMap } = transformStylesheet(source, {
      filename: diagnosticName,
      cssModules: true,
    });
    const dtsPath = `${cssPath}.d.ts`;
    expectedDts.add(dtsPath);
    const nextContent = formatCssModuleDts(exportsMap);
    let currentContent: string | undefined;
    try {
      currentContent = await readFile(dtsPath, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    if (currentContent !== nextContent) {
      await writeFile(dtsPath, nextContent, "utf8");
    }
  }

  for (const dtsPath of existingDts) {
    if (!expectedDts.has(dtsPath)) {
      await unlink(dtsPath);
    }
  }

  return modules;
}
