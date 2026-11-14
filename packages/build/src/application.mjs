import { access, readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { workspaceRoot } from "./build.mjs";

async function exists(filename) {
  try { await access(filename); return true; }
  catch (error) { if (error.code === "ENOENT") return false; throw error; }
}

export async function resolveApplication(selector, { cwd = process.cwd(), root = workspaceRoot } = {}) {
  let directory;
  if (selector !== undefined) {
    const local = path.resolve(cwd, selector);
    // Explicit paths never fall back to a similarly named repository example.
    const explicitPath = path.isAbsolute(selector) || selector.includes("/") || selector.includes("\\") || selector === "." || selector === "..";
    directory = explicitPath || await exists(local) ? local : path.join(root, "apps/examples", selector);
  } else {
    directory = await exists(path.join(cwd, "src/main.tsx")) ? cwd :
      path.resolve(cwd) === path.resolve(root) ? path.join(root, "apps/examples/counter") : cwd;
  }
  const appDirectory = await realpath(directory).catch(error => {
    throw new Error(`Cannot open application directory ${directory}: ${error.message}`, { cause: error });
  });
  for (const filename of ["src/main.tsx", "tsconfig.json"]) {
    if (!await exists(path.join(appDirectory, filename))) throw new Error(`Application ${appDirectory} requires ${filename}`);
  }
  let name = path.basename(appDirectory);
  try {
    const manifest = JSON.parse(await readFile(path.join(appDirectory, "package.json"), "utf8"));
    if (path.dirname(appDirectory) !== path.join(root, "apps/examples") && typeof manifest.name === "string" && manifest.name) {
      name = manifest.name.split("/").at(-1);
    }
  } catch (error) { if (error.code !== "ENOENT") throw error; }
  // Package/directory names may contain dots, spaces, or start with a digit;
  // the native manifest identity uses a portable, deterministic identifier.
  let app = name.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
  if (!/^[a-z]/.test(app)) app = `app-${app || "local"}`;
  return { app, appDirectory, root };
}
