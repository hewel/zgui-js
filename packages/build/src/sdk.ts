import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const checkoutRoot = fileURLToPath(new URL("../../../", import.meta.url));
export const packagedSdkRoot = fileURLToPath(new URL("../sdk", import.meta.url));
export const workspaceRoot =
  import.meta.dir === path.join(checkoutRoot, "packages/build/src") &&
  existsSync(path.join(checkoutRoot, "Cargo.toml"))
    ? checkoutRoot
    : packagedSdkRoot;
