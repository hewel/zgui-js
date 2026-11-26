import { existsSync, readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { resolveNativeHost } from "@hewel/zgui-build";
import { platformHostPackageName, platformId } from "@hewel/zgui-build/internal";

export type DoctorCheckStatus = "ok" | "warn" | "fail";

export interface DoctorCheck {
  id: string;
  label: string;
  status: DoctorCheckStatus;
  detail: string;
  remediation?: string;
}

export interface DoctorReport {
  schemaVersion: 1;
  ok: boolean;
  platform: string;
  checks: DoctorCheck[];
}

export interface DoctorOptions {
  cwd?: string;
  hostRoot?: string;
  hostExecutable?: string;
  env?: Record<string, string | undefined>;
  searchFrom?: readonly string[];
}

function compareSemver(a: string, b: string): number {
  const pa = a.split(".").map(part => Number.parseInt(part, 10) || 0);
  const pb = b.split(".").map(part => Number.parseInt(part, 10) || 0);
  for (let i = 0; i < 3; i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

function resolveToolVersion(pkgName: string, searchBases: readonly string[]): string | undefined {
  for (const base of searchBases) {
    try {
      const anchor = existsSync(path.join(base, "package.json"))
        ? path.join(base, "package.json")
        : base;
      const req = createRequire(anchor);
      const manifestPath = req.resolve(`${pkgName}/package.json`);
      const parsed = JSON.parse(readFileSync(manifestPath, "utf8")) as { version?: unknown };
      if (typeof parsed.version === "string") return parsed.version;
    } catch {
      continue;
    }
  }
  return undefined;
}

function findExecutableOnPath(name: string, pathEnv: string | undefined): string | undefined {
  if (!pathEnv) return undefined;
  for (const entry of pathEnv.split(path.delimiter)) {
    if (!entry) continue;
    const candidate = path.join(entry, name);
    if (existsSync(candidate)) return candidate;
  }
  return undefined;
}

function collectVulkanIcds(env: Record<string, string | undefined>): string[] {
  const found: string[] = [];
  const raw = env.VK_DRIVER_FILES ?? env.VK_ICD_FILENAMES;
  if (raw) {
    for (const entry of raw.split(path.delimiter)) {
      if (entry && existsSync(entry)) found.push(entry);
    }
  }
  for (const dir of ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]) {
    if (!existsSync(dir)) continue;
    try {
      for (const file of readdirSync(dir)) {
        if (file.endsWith(".json")) found.push(path.join(dir, file));
      }
    } catch {
      // Ignore unreadable system directories.
    }
  }
  return [...new Set(found)];
}

export async function runDoctor({
  cwd = process.cwd(),
  hostRoot,
  hostExecutable,
  env = process.env,
  searchFrom = [],
}: DoctorOptions = {}): Promise<DoctorReport> {
  const checks: DoctorCheck[] = [];
  const currentPlatform = platformId();
  const resolvedCwd = path.resolve(cwd);
  const searchBases = [...searchFrom, resolvedCwd, import.meta.url];

  // 1. Bun runtime version.
  const minBun = "1.4.4";
  const bunOk = compareSemver(Bun.version, minBun) >= 0;
  checks.push({
    id: "bun",
    label: "Bun runtime",
    status: bunOk ? "ok" : "fail",
    detail: `Bun ${Bun.version} (required >= ${minBun})`,
    ...(bunOk
      ? {}
      : { remediation: `Upgrade Bun to ${minBun} or enter the repository nix-shell.` }),
  });

  // 2. zgui-host resolution + ABI handshake.
  let hostCheck: DoctorCheck;
  try {
    const resolved = await resolveNativeHost({
      hostExecutable,
      hostRoot,
      cwd: resolvedCwd,
      env,
      searchFrom: [import.meta.url, ...searchFrom],
      verifyAbi: true,
      allowCargoBuild: false,
    });
    const abi = resolved.abi;
    if (!abi) throw new Error("Resolved zgui-host did not report ABI metadata");
    const abiSummary = `v${abi.version} hostApiVersion=${abi.hostApiVersion} schemaVersion=${abi.schemaVersion} (${abi.platform})`;
    hostCheck = {
      id: "zgui-host",
      label: "zgui-host binary & ABI",
      status: "ok",
      detail: `${resolved.executable} [${resolved.source}] ${abiSummary}`,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const isUnbuiltCargo = message.includes("has not been built yet");
    const expectedPkg = platformHostPackageName();
    hostCheck = {
      id: "zgui-host",
      label: "zgui-host binary & ABI",
      status: isUnbuiltCargo ? "warn" : "fail",
      detail: message.split("\n")[0] ?? message,
      remediation: isUnbuiltCargo
        ? "Run `cargo build -p zgui-host` or launch `zgui-js dev` to build the local Cargo host."
        : expectedPkg
          ? `Install ${expectedPkg}, set ZGUI_JS_HOST_EXECUTABLE, or pass --host-executable / --host-root.`
          : "Pass --host-executable <PATH> or --host-root <DIR> pointing to a zgui-js checkout.",
    };
  }
  checks.push(hostCheck);

  // 3. Wayland display socket.
  const waylandDisplay = env.WAYLAND_DISPLAY?.trim();
  const runtimeDir = env.XDG_RUNTIME_DIR?.trim();
  const waylandSocket =
    waylandDisplay && runtimeDir ? path.join(runtimeDir, waylandDisplay) : undefined;
  if (waylandSocket && existsSync(waylandSocket)) {
    checks.push({
      id: "wayland",
      label: "Wayland display",
      status: "ok",
      detail: `${waylandDisplay} (${waylandSocket})`,
    });
  } else {
    checks.push({
      id: "wayland",
      label: "Wayland display",
      status: "warn",
      detail: waylandDisplay
        ? `WAYLAND_DISPLAY=${waylandDisplay} set, but socket ${waylandSocket ?? "(XDG_RUNTIME_DIR unset)"} was not found`
        : "WAYLAND_DISPLAY is not set (desktop windowing unavailable; headless preview remains usable)",
      remediation: "Run inside a Wayland compositor session for desktop `zgui-js dev`.",
    });
  }

  // 4. Vulkan / Mesa ICDs.
  const icds = collectVulkanIcds(env);
  if (icds.length > 0) {
    checks.push({
      id: "vulkan",
      label: "Vulkan / Mesa ICDs",
      status: "ok",
      detail: `${icds.length} ICD manifest(s) detected (${icds.slice(0, 2).join(", ")}${icds.length > 2 ? ", ..." : ""})`,
    });
  } else {
    checks.push({
      id: "vulkan",
      label: "Vulkan / Mesa ICDs",
      status: "warn",
      detail:
        "No Vulkan ICD manifests found in VK_DRIVER_FILES, VK_ICD_FILENAMES, or /usr/share/vulkan/icd.d",
      remediation: "Enter `nix-shell` or install Mesa Vulkan drivers / set VK_DRIVER_FILES.",
    });
  }

  // 5. XDG desktop portal reachability.
  const dbusAddress = env.DBUS_SESSION_BUS_ADDRESS?.trim();
  const portalBin =
    findExecutableOnPath("xdg-desktop-portal", env.PATH) ??
    (existsSync("/usr/libexec/xdg-desktop-portal")
      ? "/usr/libexec/xdg-desktop-portal"
      : undefined) ??
    (existsSync("/usr/lib/xdg-desktop-portal") ? "/usr/lib/xdg-desktop-portal" : undefined);
  if (dbusAddress && portalBin) {
    checks.push({
      id: "xdg-portal",
      label: "XDG desktop portal",
      status: "ok",
      detail: `D-Bus session active and portal binary found (${portalBin})`,
    });
  } else {
    checks.push({
      id: "xdg-portal",
      label: "XDG desktop portal",
      status: "warn",
      detail: !dbusAddress
        ? "DBUS_SESSION_BUS_ADDRESS is unset (file picker portals unavailable in headless shells)"
        : "DBUS_SESSION_BUS_ADDRESS is set, but xdg-desktop-portal binary was not detected in standard paths",
      remediation:
        "Ensure a desktop D-Bus session and xdg-desktop-portal backend are running for file dialogs.",
    });
  }

  // 6. tsrs typechecker.
  const tsrsVersion = resolveToolVersion("tsrs", [
    ...searchBases,
    path.dirname(createRequire(import.meta.url).resolve("@hewel/zgui-build")),
  ]);
  checks.push({
    id: "tsrs",
    label: "TypeScript checker (tsrs)",
    status: tsrsVersion ? "ok" : "fail",
    detail: tsrsVersion ? `tsrs ${tsrsVersion}` : "tsrs could not be resolved",
    ...(tsrsVersion ? {} : { remediation: "Install `tsrs` or `@hewel/zgui-build` dependencies." }),
  });

  // 7. oxlint linter & eslint-plugin-solid.
  const oxlintSearchBases = [...searchBases, path.resolve(import.meta.dir, "../../..")];
  const oxlintVersion = resolveToolVersion("oxlint", oxlintSearchBases);
  const solidPluginVersion = resolveToolVersion("eslint-plugin-solid", oxlintSearchBases);
  const lintReady = Boolean(oxlintVersion && solidPluginVersion);
  const lintDetail = lintReady
    ? `oxlint ${oxlintVersion}, eslint-plugin-solid ${solidPluginVersion}`
    : oxlintVersion
      ? `oxlint ${oxlintVersion} (eslint-plugin-solid not installed in this project)`
      : solidPluginVersion
        ? `oxlint not installed in this project (eslint-plugin-solid ${solidPluginVersion})`
        : "oxlint and eslint-plugin-solid not installed in this project";
  checks.push({
    id: "oxlint",
    label: "Linter (oxlint)",
    status: lintReady ? "ok" : "warn",
    detail: lintDetail,
    ...(lintReady
      ? {}
      : {
          remediation:
            "Install `oxlint` and `eslint-plugin-solid`, and extend `@hewel/zgui/oxlint/recommended`.",
        }),
  });

  return {
    schemaVersion: 1,
    ok: checks.every(check => check.status !== "fail"),
    platform: currentPlatform,
    checks,
  };
}

export function formatDoctorReport(report: DoctorReport): string {
  const lines = [`zgui-js doctor (${report.platform}) — ${report.ok ? "OK" : "ISSUES FOUND"}`];
  for (const check of report.checks) {
    const tag = check.status === "ok" ? "[OK]  " : check.status === "warn" ? "[WARN]" : "[FAIL]";
    lines.push(`  ${tag} ${check.label}: ${check.detail}`);
    if (check.remediation) {
      lines.push(`         -> ${check.remediation}`);
    }
  }
  return lines.join("\n");
}
