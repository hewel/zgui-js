import { cp, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";

const fixture = fileURLToPath(new URL("./", import.meta.url));
const appDirectory = path.resolve(process.argv[2]);
if (path.dirname(appDirectory) !== path.join(workspaceRoot, "target")) {
  throw new Error("Native control test output must be directly below target/");
}
await mkdir(appDirectory, { recursive: true });
await cp(path.join(fixture, "src"), path.join(appDirectory, "src"), { recursive: true });
await cp(path.join(fixture, "tsconfig.json"), path.join(appDirectory, "tsconfig.json"));
const { dist } = await buildApplication("native-controls", { appDirectory });
console.log(dist);
