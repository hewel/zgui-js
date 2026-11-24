export { workspaceRoot, packagedSdkRoot } from "./sdk.ts";
export { buildSmoke, resolveSmokeTsconfigPath } from "./smoke.ts";
export { typecheckProject } from "./typecheck.ts";
export {
  buildSdkPackages,
  ensureSdkPackagesBuilt,
  hasCheckoutSdkSources,
  type SdkPackageName,
} from "./sdk-packages.ts";
export { extractStaticComponentSheet, type StaticComponentSheet } from "./compiler.ts";
export {
  EXPECTED_HOST_API_VERSION,
  EXPECTED_SCHEMA_VERSION,
  SUPPORTED_HOST_PACKAGES,
  platformHostPackageName,
  platformId,
  stageHostPackage,
  verifyHostPackage,
  type StagedHostManifest,
} from "./host.ts";
