import path from "node:path";
import { readFileSync, realpathSync } from "node:fs";
import { createHash } from "node:crypto";

export const sha256 = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
export function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

const inside = (directory, filename) => {
  const relative = path.relative(directory, filename);
  return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
};
const posix = filename => filename.split(path.sep).join("/");

// Resolve links before assigning identities, so pnpm symlinks never manufacture two origins.
export function moduleIdentity(filename, { root, appDirectory }) {
  const absolute = realpathSync(filename);
  const application = realpathSync(appDirectory);
  const appRelative = path.relative(application, absolute);
  if (inside(application, absolute) && !appRelative.split(path.sep).includes("node_modules")) return `app/${posix(appRelative)}`;
  for (let directory = path.dirname(absolute);;) {
    try {
      const pkg = JSON.parse(readFileSync(path.join(directory, "package.json"), "utf8"));
      if (typeof pkg.name === "string" && pkg.name) {
        return `packages/${pkg.name}/${posix(path.relative(directory, absolute))}`;
      }
    } catch (error) { if (error.code !== "ENOENT") throw error; }
    const parent = path.dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  if (inside(root, absolute)) return `workspace/${posix(path.relative(root, absolute))}`;
  throw new Error(`Cannot establish package-relative module identity: ${filename}`);
}

