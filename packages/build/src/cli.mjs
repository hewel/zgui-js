import path from "node:path";
import { buildApplication, workspaceRoot } from "./build.mjs";
import { buildSmoke } from "./smoke.mjs";
import { typecheckProject } from "./typecheck.mjs";
import { watchApplication } from "./watch.mjs";

const args = process.argv.slice(2);
try {
  if (args.length === 1 && args[0] === "--smoke") {
    typecheckProject(path.join(workspaceRoot, "packages/build/tsconfig.smoke.json"));
    await buildSmoke();
  } else {
    const watching = args[0] === "--watch";
    if (watching) args.shift();
    if (args.length !== 2 || args[0] !== "--app" || !["counter", "lists"].includes(args[1])) {
      throw new Error("Usage: pnpm ui:build --app counter|lists | --smoke; pnpm ui:dev --app counter|lists");
    }
    if (watching) {
      const watcher = watchApplication(args[1]);
      const close = async () => { await watcher.close(); };
      process.once("SIGINT", close);
      process.once("SIGTERM", close);
    } else {
      const { dist, manifest } = await buildApplication(args[1]);
      console.log(`Published ${path.relative(workspaceRoot, dist)} ${manifest.buildId}`);
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
