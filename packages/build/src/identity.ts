import path from "node:path";
import { readFileSync, realpathSync } from "node:fs";

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

export function sha256(bytes: string | Uint8Array): string {
  return `sha256:${new Bun.CryptoHasher("sha256").update(bytes).digest("hex")}`;
}

export function canonicalJson(value: JsonValue): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map(key => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

const inside = (directory: string, filename: string) => {
  const relative = path.relative(directory, filename);
  return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
};
const posix = (filename: string) => filename.split(path.sep).join("/");

// Resolve links before assigning identities so symlinks cannot manufacture two origins.
export function moduleIdentity(
  filename: string,
  { root, appDirectory }: { root: string; appDirectory: string },
): string {
  const absolute = realpathSync(filename);
  const application = realpathSync(appDirectory);
  const appRelative = path.relative(application, absolute);
  if (inside(application, absolute) && !appRelative.split(path.sep).includes("node_modules"))
    return `app/${posix(appRelative)}`;
  for (let directory = path.dirname(absolute); ;) {
    try {
      const pkg = JSON.parse(readFileSync(path.join(directory, "package.json"), "utf8")) as {
        name?: unknown;
      };
      if (typeof pkg.name === "string" && pkg.name) {
        return `packages/${pkg.name}/${posix(path.relative(directory, absolute))}`;
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const parent = path.dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  if (inside(root, absolute)) return `workspace/${posix(path.relative(root, absolute))}`;
  throw new Error(`Cannot establish package-relative module identity: ${filename}`);
}
