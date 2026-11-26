import { readFile } from "node:fs/promises";
import path from "node:path";
import type {
  BuildApplicationOptions,
  Capabilities,
  FontDeclaration,
  Manifest,
  WindowDeclaration,
} from "./index.ts";
import { resourcePath } from "./publication.ts";

type CapabilityOverlays = Pick<BuildApplicationOptions, "extraCapabilities" | "capabilitiesFile">;

/** Read author declarations and build-time grants before compiling and sealing a package. */
export async function readManifestPolicy(
  app: string,
  appDirectory: string,
  assets: ReadonlyMap<string, Uint8Array>,
  overlays: CapabilityOverlays,
): Promise<
  Pick<Manifest, "id" | "window" | "capabilities" | "fonts"> & {
    workerEntries: { name: string; entry: string }[];
  }
> {
  const text = await readFile(path.join(appDirectory, "package.json"), "utf8").catch(
    (error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return undefined;
      throw error;
    },
  );
  const pkg =
    text === undefined
      ? undefined
      : (JSON.parse(text) as {
          zgui?: {
            id?: unknown;
            window?: unknown;
            capabilities?: unknown;
            workers?: unknown;
            fonts?: unknown;
          };
        });
  const zgui = pkg?.zgui;
  const id = zgui?.id === undefined ? `dev.example.${app}` : zgui.id;
  requireManifestText(id, "zgui.id");
  const window = parseWindowDeclaration(zgui?.window);
  const capabilities = await mergeCapabilities(zgui?.capabilities, overlays);
  const workerEntries = parseWorkerDeclarations(zgui?.workers, appDirectory);
  const fonts = parseFonts(zgui?.fonts, assets);
  return {
    id,
    workerEntries,
    fonts,
    ...(window ? { window } : {}),
    ...(capabilities ? { capabilities } : {}),
  };
}

function parseFonts(input: unknown, assets: ReadonlyMap<string, Uint8Array>): FontDeclaration[] {
  const declarations = input ?? [];
  if (!Array.isArray(declarations))
    throw new Error("package.json zgui.fonts must be an array of {family,path}");
  const fonts: FontDeclaration[] = [];
  const families = new Set<string>();
  for (const font of declarations as unknown[]) {
    if (!isFontDeclaration(font))
      throw new Error("Invalid application font mapping; expected {family,path}");
    resourcePath(font.path);
    if (!assets.has(font.path)) throw new Error(`Font path must be a declared asset: ${font.path}`);
    const family = font.family.toLowerCase();
    if (families.has(family)) throw new Error(`Duplicate application font family: ${font.family}`);
    families.add(family);
    fonts.push(font);
  }
  return fonts;
}

// Match Rust char::is_whitespace plus BOM for the two trimmed manifest fields.
const manifestTextEdge = /^[\p{White_Space}\uFEFF]|[\p{White_Space}\uFEFF]$/u;

function requireManifestText(value: unknown, field: string): asserts value is string {
  if (!capabilityString(value) || manifestTextEdge.test(value)) {
    throw new Error(
      `package.json ${field} must be a non-empty, trimmed string without control characters`,
    );
  }
}

const HOST_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,63}(?![\s\S])$/;

function isFontDeclaration(font: unknown): font is FontDeclaration {
  return (
    !!font &&
    typeof font === "object" &&
    "family" in font &&
    typeof font.family === "string" &&
    !!font.family.trim() &&
    font.family === font.family.trim() &&
    "path" in font &&
    typeof font.path === "string" &&
    Object.keys(font).every(key => key === "family" || key === "path")
  );
}

/** Reject non-scalar strings and controls. Commands may contain significant non-ASCII spacing. */
function capabilityString(value: unknown): value is string {
  if (typeof value !== "string") return false;
  let nonspace = false;
  for (const char of value) {
    const code = char.charCodeAt(0);
    if (
      code < 32 ||
      (code >= 127 && code <= 159) ||
      (char.length === 1 && code >= 0xd800 && code <= 0xdfff)
    )
      return false;
    nonspace ||= code !== 32;
  }
  return nonspace;
}

function capabilityPath(value: string, kind: "host" | "directory"): boolean {
  if (value === "/") return true;
  if (value.includes("\\")) return false;
  const [root, ...parts] = value.split("/");
  if (
    root !== "" &&
    root !== "$HOME" &&
    root !== "$XDG_DATA_HOME" &&
    root !== (kind === "host" ? "$XDG_RUNTIME_DIR" : "$XDG_DOCUMENTS_DIR")
  ) {
    return false;
  }
  return parts.every(part => {
    if (part === "" || part === "." || part === "..") return false;
    if (root === "") return true; // Literal paths do not substitute variables.
    if (kind === "directory") return !part.includes("$");
    return part === "$WAYLAND_DISPLAY" || !part.replaceAll("${WAYLAND_DISPLAY}", "").includes("$");
  });
}

function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".");
  if (parts.length !== 4 || !parts.every(p => /^\d{1,3}$/.test(p))) return false;
  const nums = parts.map(Number);
  if (nums.some(n => n > 255)) return false;
  const [a, b, c, d] = nums as [number, number, number, number];
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a >= 224 && a <= 239) ||
    (a === 255 && b === 255 && c === 255 && d === 255) ||
    (a === 192 && b === 0 && c === 2) ||
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113)
  );
}

function isPrivateHost(hostname: string): boolean {
  if (hostname.startsWith("[")) {
    const bare = hostname.slice(1, -1);
    if (bare.startsWith("::ffff:")) return isPrivateIpv4(bare.slice("::ffff:".length));
    if (bare === "::1" || bare === "::") return true;
    const head = Number.parseInt(bare, 16);
    return (head & 0xfe00) === 0xfc00 || (head & 0xffc0) === 0xfe80 || (head & 0xff00) === 0xff00;
  }
  const bare = hostname.replace(/\.+$/, "");
  return bare === "localhost" || bare.endsWith(".localhost") || isPrivateIpv4(bare);
}

function parseNetworkOrigin(
  entry: unknown,
  field: string,
): {
  canonical: string;
  insecure: boolean;
  host: string;
} {
  if (
    !capabilityString(entry) ||
    /[\p{White_Space}\uFEFF*]/u.test(entry) ||
    !entry.includes("://")
  ) {
    throw new Error(`${field} contains invalid origin ${JSON.stringify(entry)}`);
  }
  let parsed: URL;
  try {
    parsed = new URL(entry);
  } catch {
    throw new Error(`${field} contains invalid origin ${JSON.stringify(entry)}`);
  }
  const scheme = parsed.protocol.slice(0, -1).toLowerCase();
  if (scheme !== "https" && scheme !== "http" && scheme !== "wss" && scheme !== "ws") {
    throw new Error(`${field} origin scheme must be https, http, wss, or ws: ${entry}`);
  }
  const authority = entry.slice(entry.indexOf("://") + 3);
  if (authority.includes("@")) {
    throw new Error(`${field} origin must not include userinfo: ${entry}`);
  }
  if (/[/\\?#]/.test(authority)) {
    throw new Error(`${field} origin must not include a path, query, or fragment: ${entry}`);
  }
  let host = parsed.hostname.toLowerCase();
  if (!host) {
    throw new Error(`${field} origin must include a host: ${entry}`);
  }
  // WHATWG URL uses hex for mapped addresses; Rust's Ipv6Addr displays dotted IPv4.
  const mapped = /^\[::ffff:([a-f0-9]{1,4}):([a-f0-9]{1,4})\]$/.exec(host);
  if (mapped) {
    const high = Number.parseInt(mapped[1]!, 16);
    const low = Number.parseInt(mapped[2]!, 16);
    host = `[::ffff:${high >> 8}.${high & 255}.${low >> 8}.${low & 255}]`;
  }
  const defaultPort = scheme === "https" || scheme === "wss" ? 443 : 80;
  const port = parsed.port ? Number(parsed.port) : defaultPort;
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`${field} origin has invalid port: ${entry}`);
  }
  const canonical = port === defaultPort ? `${scheme}://${host}` : `${scheme}://${host}:${port}`;
  return { canonical, insecure: scheme === "http" || scheme === "ws", host };
}

/** Validate each source before any of its grants can enter the union. */
function parseCapabilities(input: unknown, source: string): Capabilities {
  if (input !== undefined && (input === null || typeof input !== "object" || Array.isArray(input)))
    throw new Error(`${source} must be an object`);

  const capabilities = Object.create(null) as Capabilities;
  for (const [key, value] of Object.entries(input ?? {}) as [string, unknown][]) {
    const field = `${source}.${key}`;
    switch (key) {
      case "shellSurfaces":
      case "sessionLock":
      case "controlChannel":
        if (typeof value !== "boolean") throw new Error(`${field} must be a boolean`);
        if (value) capabilities[key] = true;
        break;
      case "clipboard": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "read") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (typeof subVal !== "boolean") throw new Error(`${field}.read must be a boolean`);
          if (subVal) capabilities.clipboard = { read: true };
        }
        break;
      }
      case "sqlite": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "databases") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (!Array.isArray(subVal)) throw new Error(`${field}.databases must be an array`);
          const names = new Set<string>();
          for (const item of subVal as unknown[]) {
            if (typeof item !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(item))
              throw new Error(`${field}.databases contains invalid entries`);
            if (names.has(item)) throw new Error(`${field}.databases contains duplicates`);
            names.add(item);
          }
          if (names.size) capabilities.sqlite = { databases: [...names] };
        }
        break;
      }
      case "files": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "directories") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (!Array.isArray(subVal)) throw new Error(`${field}.directories must be an array`);
          const seen = new Set<string>();
          const dirs: { path: string; mode: "read" | "readwrite" }[] = [];
          for (const item of subVal as unknown[]) {
            if (!item || typeof item !== "object" || Array.isArray(item))
              throw new Error(`${field}.directories entries must be objects`);
            const record = item as Record<string, unknown>;
            const keys = Object.keys(record);
            if (!keys.every(k => k === "path" || k === "mode"))
              throw new Error(`${field}.directories entry contains unknown keys`);
            if (
              !capabilityString(record.path) ||
              !capabilityPath(record.path, "directory") ||
              (record.mode !== "read" && record.mode !== "readwrite")
            )
              throw new Error(`${field}.directories contains invalid entries`);
            if (seen.has(record.path))
              throw new Error(`${field}.directories contains duplicate path ${record.path}`);
            seen.add(record.path);
            dirs.push({ path: record.path, mode: record.mode });
          }
          if (dirs.length) capabilities.files = { directories: dirs };
        }
        break;
      }
      case "network": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        const network: {
          origins?: string[];
          originRules?: {
            origin: string;
            allowInsecureHttp?: boolean;
            allowPrivateNetworks?: boolean;
          }[];
          allowInsecureHttp?: boolean;
          allowPrivateNetworks?: boolean;
        } = {};
        const originSet = new Set<string>();
        const ruleSet = new Set<string>();
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          switch (subKey) {
            case "origins": {
              if (!Array.isArray(subVal)) throw new Error(`${field}.origins must be an array`);
              for (const item of subVal as unknown[]) {
                const parsed = parseNetworkOrigin(item, `${field}.origins`);
                if (originSet.has(parsed.canonical))
                  throw new Error(`${field}.origins contains duplicate origin ${parsed.canonical}`);
                originSet.add(parsed.canonical);
              }
              if (originSet.size) network.origins = [...originSet];
              break;
            }
            case "originRules": {
              if (!Array.isArray(subVal)) throw new Error(`${field}.originRules must be an array`);
              const rules: {
                origin: string;
                allowInsecureHttp?: boolean;
                allowPrivateNetworks?: boolean;
              }[] = [];
              for (const item of subVal as unknown[]) {
                if (!item || typeof item !== "object" || Array.isArray(item))
                  throw new Error(`${field}.originRules entries must be objects`);
                const record = item as Record<string, unknown>;
                for (const ruleKey of Object.keys(record)) {
                  if (
                    ruleKey !== "origin" &&
                    ruleKey !== "allowInsecureHttp" &&
                    ruleKey !== "allowPrivateNetworks"
                  ) {
                    throw new Error(`${field}.originRules entry contains unknown key: ${ruleKey}`);
                  }
                }
                const parsed = parseNetworkOrigin(record.origin, `${field}.originRules`);
                if (
                  record.allowInsecureHttp !== undefined &&
                  typeof record.allowInsecureHttp !== "boolean"
                ) {
                  throw new Error(`${field}.originRules allowInsecureHttp must be a boolean`);
                }
                if (
                  record.allowPrivateNetworks !== undefined &&
                  typeof record.allowPrivateNetworks !== "boolean"
                ) {
                  throw new Error(`${field}.originRules allowPrivateNetworks must be a boolean`);
                }
                const allowInsecureHttp = record.allowInsecureHttp === true;
                const allowPrivateNetworks = record.allowPrivateNetworks === true;
                if (parsed.insecure && !allowInsecureHttp) {
                  throw new Error(
                    `${field}.originRules contains insecure origin ${parsed.canonical} without allowInsecureHttp: true`,
                  );
                }
                if (isPrivateHost(parsed.host) && !allowPrivateNetworks) {
                  throw new Error(
                    `${field}.originRules contains private/loopback origin ${parsed.canonical} without allowPrivateNetworks: true`,
                  );
                }
                if (ruleSet.has(parsed.canonical)) {
                  throw new Error(
                    `${field}.originRules contains duplicate origin ${parsed.canonical}`,
                  );
                }
                ruleSet.add(parsed.canonical);
                rules.push({
                  origin: parsed.canonical,
                  ...(allowInsecureHttp ? { allowInsecureHttp: true } : {}),
                  ...(allowPrivateNetworks ? { allowPrivateNetworks: true } : {}),
                });
              }
              if (rules.length) network.originRules = rules;
              break;
            }
            case "allowInsecureHttp":
            case "allowPrivateNetworks": {
              if (typeof subVal !== "boolean")
                throw new Error(`${field}.${subKey} must be a boolean`);
              if (subVal) network[subKey] = true;
              break;
            }
            default:
              throw new Error(`Unknown capability: ${field}.${subKey}`);
          }
        }
        for (const origin of ruleSet) {
          if (originSet.has(origin)) {
            throw new Error(
              `${field} declares duplicate origin in origins and originRules: ${origin}`,
            );
          }
        }
        if (Object.keys(network).length) capabilities.network = network;
        break;
      }
      case "imagePrefixes":
      case "unixSockets":
      case "commands":
      case "namedSockets": {
        if (!Array.isArray(value)) throw new Error(`${field} must be an array`);
        const entries = new Set<string>();
        for (const item of value as unknown[]) {
          if (
            !capabilityString(item) ||
            (key === "namedSockets" && !HOST_NAME.test(item)) ||
            ((key === "imagePrefixes" || key === "unixSockets") && !capabilityPath(item, "host"))
          )
            throw new Error(`${field} contains invalid entries`);
          if (entries.has(item)) throw new Error(`${field} contains duplicates`);
          entries.add(item);
        }
        if (entries.size) capabilities[key] = [...entries];
        break;
      }
      default:
        throw new Error(`Unknown capability: ${field}`);
    }
  }
  return capabilities;
}

/**
 * Build-time overlays may grant `shellSurfaces: true`, sealing that grant into the package.
 * Only the base package may grant `sessionLock: true`. Runtime `--extensions-overlay` is
 * intentionally stricter: it may escalate neither shell surfaces nor session lock.
 */
function capabilityOverlay(input: unknown, source: string): Capabilities {
  if (
    input !== null &&
    typeof input === "object" &&
    "sessionLock" in input &&
    input.sessionLock === true
  )
    throw new Error(`${source}.sessionLock: true may only be declared in base package.json`);
  return parseCapabilities(input, source);
}

async function mergeCapabilities(
  input: unknown,
  { extraCapabilities, capabilitiesFile }: CapabilityOverlays,
): Promise<Capabilities | undefined> {
  const base = parseCapabilities(input, "package.json zgui.capabilities");
  const sources = [base, capabilityOverlay(extraCapabilities, "extraCapabilities")];
  if (capabilitiesFile !== undefined) {
    const filename = path.resolve(capabilitiesFile);
    const source = `capabilitiesFile (${filename})`;
    let overlay: unknown;
    try {
      overlay = JSON.parse(await readFile(filename, "utf8"));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new Error(`${source}: cannot read JSON: ${message}`, { cause: error });
    }
    sources.push(capabilityOverlay(overlay, source));
  }
  const merged: Capabilities = {};
  for (const key of ["shellSurfaces", "controlChannel"] as const)
    if (sources.some(source => source[key])) merged[key] = true;
  if (base.sessionLock) merged.sessionLock = true;
  for (const key of ["imagePrefixes", "unixSockets", "namedSockets", "commands"] as const) {
    const union = new Set(sources.flatMap(source => source[key] ?? []));
    if (union.size) merged[key] = [...union].sort();
  }
  if (sources.some(source => source.clipboard?.read)) merged.clipboard = { read: true };
  const sqliteDatabases = new Set(sources.flatMap(source => source.sqlite?.databases ?? []));
  if (sqliteDatabases.size) merged.sqlite = { databases: [...sqliteDatabases].sort() };
  const fileDirs = new Map<string, "read" | "readwrite">();
  for (const grant of sources.flatMap(source => source.files?.directories ?? [])) {
    const existing = fileDirs.get(grant.path);
    if (!existing || grant.mode === "readwrite") fileDirs.set(grant.path, grant.mode);
  }
  if (fileDirs.size) {
    merged.files = {
      directories: [...fileDirs.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([dirPath, mode]) => ({ path: dirPath, mode })),
    };
  }
  const networkSources = sources
    .map(source => source.network)
    .filter((net): net is NonNullable<Capabilities["network"]> => net !== undefined);
  const usesLegacySingleSource =
    networkSources.length === 1 &&
    (networkSources[0]!.originRules === undefined || networkSources[0]!.originRules.length === 0);
  if (usesLegacySingleSource) {
    const only = networkSources[0]!;
    const allowInsecureHttp = Boolean(only.allowInsecureHttp);
    const allowPrivateNetworks = Boolean(only.allowPrivateNetworks);
    const networkOrigins = [...new Set(only.origins ?? [])].sort();
    for (const origin of networkOrigins) {
      const parsed = parseNetworkOrigin(origin, "capabilities.network.origins");
      if (parsed.insecure && !allowInsecureHttp) {
        throw new Error(
          `capabilities.network.origins contains insecure origin ${origin} without allowInsecureHttp: true`,
        );
      }
      if (isPrivateHost(parsed.host) && !allowPrivateNetworks) {
        throw new Error(
          `capabilities.network.origins contains private/loopback origin ${origin} without allowPrivateNetworks: true`,
        );
      }
    }
    if (networkOrigins.length || allowInsecureHttp || allowPrivateNetworks) {
      merged.network = {
        ...(networkOrigins.length ? { origins: networkOrigins } : {}),
        ...(allowInsecureHttp ? { allowInsecureHttp: true } : {}),
        ...(allowPrivateNetworks ? { allowPrivateNetworks: true } : {}),
      };
    }
  } else if (networkSources.length > 0) {
    const byOrigin = new Map<
      string,
      { allowInsecureHttp: boolean; allowPrivateNetworks: boolean }
    >();
    for (const net of networkSources) {
      const sourceInsecure = Boolean(net.allowInsecureHttp);
      const sourcePrivate = Boolean(net.allowPrivateNetworks);
      for (const origin of net.origins ?? []) {
        const parsed = parseNetworkOrigin(origin, "capabilities.network.origins");
        if (parsed.insecure && !sourceInsecure) {
          throw new Error(
            `capabilities.network.origins contains insecure origin ${origin} without allowInsecureHttp: true`,
          );
        }
        if (isPrivateHost(parsed.host) && !sourcePrivate) {
          throw new Error(
            `capabilities.network.origins contains private/loopback origin ${origin} without allowPrivateNetworks: true`,
          );
        }
        const prev = byOrigin.get(parsed.canonical) ?? {
          allowInsecureHttp: false,
          allowPrivateNetworks: false,
        };
        byOrigin.set(parsed.canonical, {
          allowInsecureHttp: prev.allowInsecureHttp || sourceInsecure,
          allowPrivateNetworks: prev.allowPrivateNetworks || sourcePrivate,
        });
      }
      for (const rule of net.originRules ?? []) {
        const parsed = parseNetworkOrigin(rule.origin, "capabilities.network.originRules");
        const prev = byOrigin.get(parsed.canonical) ?? {
          allowInsecureHttp: false,
          allowPrivateNetworks: false,
        };
        byOrigin.set(parsed.canonical, {
          allowInsecureHttp: prev.allowInsecureHttp || Boolean(rule.allowInsecureHttp),
          allowPrivateNetworks: prev.allowPrivateNetworks || Boolean(rule.allowPrivateNetworks),
        });
      }
    }
    if (byOrigin.size > 0) {
      merged.network = {
        originRules: [...byOrigin.entries()]
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([origin, flags]) => ({
            origin,
            ...(flags.allowInsecureHttp ? { allowInsecureHttp: true } : {}),
            ...(flags.allowPrivateNetworks ? { allowPrivateNetworks: true } : {}),
          })),
      };
    }
  }
  return Object.keys(merged).length ? merged : undefined;
}

const WINDOW_KEYS = new Set(["title", "width", "height", "minWidth", "minHeight", "resizable"]);

function parseWindowDeclaration(input: unknown): WindowDeclaration | undefined {
  if (input === undefined) return undefined;
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("package.json zgui.window must be an object");
  }
  const record = input as Record<string, unknown>;
  for (const key of Object.keys(record)) {
    if (!WINDOW_KEYS.has(key)) throw new Error(`Unknown package.json zgui.window key: ${key}`);
  }
  const result: WindowDeclaration = {};
  if (record.title !== undefined) {
    requireManifestText(record.title, "zgui.window.title");
    result.title = record.title;
  }
  for (const key of ["width", "height", "minWidth", "minHeight"] as const) {
    const value = record[key];
    if (value !== undefined) {
      if (typeof value !== "number" || !Number.isInteger(value) || value < 1 || value > 16384) {
        throw new Error(`package.json zgui.window.${key} must be an integer between 1 and 16384`);
      }
      result[key] = value;
    }
  }
  if (record.resizable !== undefined) {
    if (typeof record.resizable !== "boolean") {
      throw new Error("package.json zgui.window.resizable must be a boolean");
    }
    result.resizable = record.resizable;
  }
  return Object.keys(result).length ? result : undefined;
}

const MAX_WORKERS = 32;

function parseWorkerDeclarations(
  input: unknown,
  appDirectory: string,
): { name: string; entry: string }[] {
  if (input === undefined) return [];
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error(
      "package.json zgui.workers must be an object mapping worker names to relative entry paths",
    );
  }
  const entries = Object.entries(input as Record<string, unknown>);
  if (entries.length > MAX_WORKERS) {
    throw new Error(`package.json zgui.workers exceeds maximum of ${MAX_WORKERS} workers`);
  }
  const resolvedAppDir = path.resolve(appDirectory);
  const parsed: { name: string; entry: string }[] = [];
  for (const [name, relPath] of entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    if (!HOST_NAME.test(name)) {
      throw new Error(`Invalid compute worker name: ${name}`);
    }
    if (typeof relPath !== "string" || !relPath.trim()) {
      throw new Error(`package.json zgui.workers.${name} must be a non-empty relative path`);
    }
    resourcePath(relPath);
    const fullPath = path.resolve(resolvedAppDir, relPath);
    if (!fullPath.startsWith(`${resolvedAppDir}${path.sep}`)) {
      throw new Error(
        `package.json zgui.workers.${name} escapes application directory: ${relPath}`,
      );
    }
    parsed.push({ name, entry: fullPath });
  }
  return parsed;
}
