import { existsSync, readFileSync } from "node:fs";
import { mkdir, readdir, writeFile } from "node:fs/promises";
import path from "node:path";

function cliPackageVersion(): string {
  const pkg = JSON.parse(
    readFileSync(path.resolve(import.meta.dir, "../package.json"), "utf8"),
  ) as { version?: unknown };
  if (typeof pkg.version !== "string" || !pkg.version.trim()) {
    throw new Error("Unable to read @hewel/zgui-cli package version");
  }
  return pkg.version.trim();
}

function sdkDependencyRange(): string {
  const version = cliPackageVersion();
  return version.includes("-") ? version : `^${version}`;
}

export type InitTemplate = "desktop-app" | "wayland-shell";

export interface InitApplicationOptions {
  directory?: string;
  template?: InitTemplate;
  name?: string;
  force?: boolean;
  cwd?: string;
}

export interface InitApplicationResult {
  directory: string;
  name: string;
  template: InitTemplate;
  files: string[];
}

function sanitizeAppName(raw: string): string {
  const cleaned = raw
    .trim()
    .toLowerCase()
    .replace(/^@[^/]+\//, "")
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (!cleaned || !/^[a-z]/.test(cleaned)) {
    return `zgui-${cleaned || "app"}`;
  }
  return cleaned;
}

function titleCase(slug: string): string {
  return slug
    .split("-")
    .filter(Boolean)
    .map(part => `${part[0]!.toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function desktopAppFiles(name: string): Record<string, string> {
  const title = titleCase(name);
  const sdkRange = sdkDependencyRange();
  return {
    "package.json": `${JSON.stringify(
      {
        name,
        version: "0.1.0",
        private: true,
        type: "module",
        scripts: {
          dev: "zgui-js dev",
          build: "zgui-js build",
          preview: "zgui-js preview",
          package: "zgui-js package",
          doctor: "zgui-js doctor",
          typecheck: "tsrs --noEmit",
          lint: "oxlint --deny-warnings",
        },
        zgui: {
          id: `dev.example.${name}`,
          window: {
            title,
            width: 960,
            height: 640,
          },
        },
        dependencies: {
          "@hewel/zgui": sdkRange,
          "@hewel/zgui-services": sdkRange,
          "solid-js": "2.0.1",
        },
        devDependencies: {
          "@hewel/zgui-cli": sdkRange,
          oxlint: "1.94.0",
          tsrs: "7.1.0-beta.tsrs2",
        },
      },
      null,
      2,
    )}\n`,
    "tsconfig.json": `${JSON.stringify(
      {
        extends: "@hewel/zgui/tsconfig.native.json",
        include: ["src"],
      },
      null,
      2,
    )}\n`,
    ".oxlintrc.json": `${JSON.stringify(
      {
        extends: ["./node_modules/@hewel/zgui/oxlint/recommended.json"],
        ignorePatterns: ["dist/**", "dist-package/**", ".zgui-generations/**"],
      },
      null,
      2,
    )}\n`,
    "zgui.capabilities.json": "{}\n",
    ".gitignore": `node_modules/
dist/
dist-package/
.zgui-generations/
.zgui-build-status.json
.zgui-build-status.json.*.tmp
.zgui-native-status.json
.zgui-native-status.json.*.tmp
*.module.css.d.ts
`,
    "src/app.css": `:root {
  font-family: sans-serif;
  font-size: 16px;
}

box,
text {
  box-sizing: border-box;
}
`,
    "src/App.module.css": `.shell {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 28px;
  width: 100%;
  height: 100%;
  background: var(--zui-color-background);
  color: var(--zui-color-foreground);
}

.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px;
  border-radius: 12px;
  border: 1px solid var(--zui-color-border);
  background: var(--zui-color-surface);
}

.title {
  font-size: 24px;
  font-weight: 700;
}

.subtitle {
  font-size: 14px;
  color: var(--zui-color-muted-foreground);
}

.actions {
  display: flex;
  flex-direction: row;
  gap: 10px;
}
`,
    "src/main.tsx": `import { createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";
import styles from "./App.module.css";

export const hostApiVersion = 1;

function App() {
  const [count, setCount] = createSignal(0);

  return (
    <ThemeProvider>
      <box class={styles.shell}>
        <box class={styles.card}>
          <text class={styles.title}>${title}</text>
          <text class={styles.subtitle}>Native desktop UI built with zgui-js + CSS Modules.</text>
          <text data-testid="counter-value">Count: {count()}</text>
          <box class={styles.actions}>
            <Button data-testid="btn-inc" onClick={() => setCount(value => value + 1)}>
              Increment
            </Button>
            <Button variant="outline" onClick={() => setCount(0)}>
              Reset
            </Button>
          </box>
        </box>
      </box>
    </ThemeProvider>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}
`,
  };
}

function waylandShellFiles(name: string): Record<string, string> {
  const title = titleCase(name);
  const sdkRange = sdkDependencyRange();
  return {
    "package.json": `${JSON.stringify(
      {
        name,
        version: "0.1.0",
        private: true,
        type: "module",
        scripts: {
          dev: "zgui-js dev --pure-shell",
          build: "zgui-js build",
          preview: "zgui-js preview",
          package: "zgui-js package --pure-shell",
          doctor: "zgui-js doctor",
          typecheck: "tsrs --noEmit",
          lint: "oxlint --deny-warnings",
        },
        zgui: {
          id: `dev.example.${name}`,
          capabilities: {
            shellSurfaces: true,
            controlChannel: true,
          },
        },
        dependencies: {
          "@hewel/zgui": sdkRange,
          "@hewel/zgui-services": sdkRange,
          "solid-js": "2.0.1",
        },
        devDependencies: {
          "@hewel/zgui-cli": sdkRange,
          oxlint: "1.94.0",
          tsrs: "7.1.0-beta.tsrs2",
        },
      },
      null,
      2,
    )}\n`,
    "tsconfig.json": `${JSON.stringify(
      {
        extends: "@hewel/zgui/tsconfig.native.json",
        include: ["src"],
      },
      null,
      2,
    )}\n`,
    ".oxlintrc.json": `${JSON.stringify(
      {
        extends: ["./node_modules/@hewel/zgui/oxlint/recommended.json"],
        ignorePatterns: ["dist/**", "dist-package/**", ".zgui-generations/**"],
      },
      null,
      2,
    )}\n`,
    "zgui.capabilities.json": `${JSON.stringify(
      {
        shellSurfaces: true,
        controlChannel: true,
      },
      null,
      2,
    )}\n`,
    ".gitignore": `node_modules/
dist/
dist-package/
.zgui-generations/
.zgui-build-status.json
.zgui-build-status.json.*.tmp
.zgui-native-status.json
.zgui-native-status.json.*.tmp
*.module.css.d.ts
`,
    "src/app.css": `:root {
  font-family: sans-serif;
  font-size: 14px;
}

box,
text {
  box-sizing: border-box;
}
`,
    "src/App.module.css": `.bar {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  height: 36px;
  padding: 0 14px;
  background: rgba(20, 24, 34, 0.92);
  color: #edf2fa;
  border-bottom: 1px solid rgba(255, 255, 255, 0.12);
}

.brand {
  font-weight: 700;
}

.pill {
  padding: 4px 10px;
  border-radius: 999px;
  background: rgba(120, 160, 255, 0.22);

  &:hover {
    background: rgba(120, 160, 255, 0.35);
  }
}
`,
    "src/main.tsx": `import { createSignal, LayerSurface, renderApp, type NativeNode } from "@hewel/zgui";
import styles from "./App.module.css";

export const hostApiVersion = 1;

function ShellBar() {
  const [clicks, setClicks] = createSignal(0);

  return (
    <LayerSurface
      name="${name}-bar"
      layer="top"
      anchor={["top", "left", "right"]}
      height={36}
      exclusiveZone={36}
    >
      <box class={styles.bar}>
        <text class={styles.brand}>${title}</text>
        <box
          class={styles.pill}
          data-testid="shell-pill"
          onClick={() => setClicks(value => value + 1)}
        >
          <text>Events: {clicks()}</text>
        </box>
      </box>
    </LayerSurface>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <ShellBar />, root);
}
`,
  };
}

export async function initApplication({
  directory,
  template = "desktop-app",
  name,
  force = false,
  cwd = process.cwd(),
}: InitApplicationOptions = {}): Promise<InitApplicationResult> {
  if (template !== "desktop-app" && template !== "wayland-shell") {
    throw new Error(
      `Unknown template "${String(template)}". Expected "desktop-app" or "wayland-shell".`,
    );
  }
  const targetDir = path.resolve(cwd, directory ?? ".");
  if (existsSync(targetDir) && !force) {
    const existing = await readdir(targetDir);
    if (existing.length > 0) {
      throw new Error(
        `Target directory is not empty: ${targetDir} (pass --force to overwrite scaffold files)`,
      );
    }
  }
  const appName = sanitizeAppName(name ?? path.basename(targetDir));
  const templates =
    template === "wayland-shell" ? waylandShellFiles(appName) : desktopAppFiles(appName);

  await mkdir(targetDir, { recursive: true });
  const written: string[] = [];
  for (const [relativePath, content] of Object.entries(templates)) {
    const fullPath = path.join(targetDir, relativePath);
    await mkdir(path.dirname(fullPath), { recursive: true });
    await writeFile(fullPath, content, "utf8");
    written.push(relativePath);
  }
  return {
    directory: targetDir,
    name: appName,
    template,
    files: written.sort(),
  };
}

const initUsage = `Usage: zgui-js init [DIRECTORY] [options]
       create-zgui-js [DIRECTORY] [options]

Options:
  --template <desktop-app|wayland-shell>  Project template (default: desktop-app)
  --name <APP_NAME>                       Override package/app name
  --force                                 Allow scaffolding into a non-empty directory
  -h, --help                              Show this help message`;

export async function runInitCli(
  args: readonly string[],
  { cwd = process.cwd(), log = (line: string) => console.log(line) } = {},
): Promise<number> {
  if (args.includes("-h") || args.includes("--help")) {
    log(initUsage);
    return 0;
  }
  const remaining = [...args];
  let directory: string | undefined;
  let template: InitTemplate = "desktop-app";
  let name: string | undefined;
  let force = false;

  while (remaining.length > 0) {
    const arg = remaining.shift()!;
    if (arg === "--") continue;
    if (!arg.startsWith("-")) {
      if (directory !== undefined) {
        throw new Error("Specify at most one target directory");
      }
      directory = arg;
      continue;
    }
    if (arg === "--force") {
      force = true;
      continue;
    }
    if (arg === "--template" || arg === "-t") {
      const val = remaining.shift();
      if (!val || val.startsWith("-")) throw new Error(`${arg} requires a template name`);
      if (val !== "desktop-app" && val !== "wayland-shell") {
        throw new Error(`Unknown template "${val}". Expected "desktop-app" or "wayland-shell".`);
      }
      template = val;
      continue;
    }
    if (arg === "--name") {
      const val = remaining.shift();
      if (!val || val.startsWith("-")) throw new Error("--name requires a value");
      name = val;
      continue;
    }
    throw new Error(`Unknown option for init: ${arg}`);
  }

  const result = await initApplication({ directory, template, name, force, cwd });
  log(`Scaffolded zgui-js ${result.template} "${result.name}" in ${result.directory}`);
  log(`Created ${result.files.length} files (${result.files.join(", ")})`);
  return 0;
}
