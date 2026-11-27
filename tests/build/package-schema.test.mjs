import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { validateAgainstSchema } from "./schema-fixture.mjs";

test("package.schema.json aligns with Rust and TS capabilities", async () => {
  const schema = JSON.parse(
    await readFile(path.join(workspaceRoot, "schemas/package.schema.json"), "utf8"),
  );
  const capProps = schema.properties.capabilities.properties;
  for (const key of ["clipboard", "sqlite", "files", "systemFiles", "network", "singleInstance"]) {
    assert.ok(key in capProps, `schema missing capabilities.${key}`);
  }
  assert.ok("originRules" in capProps.network.properties, "schema missing network.originRules");

  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  const dir = await mkdtemp(path.join(workspaceRoot, "target", "zgui-schema-parity-"));
  try {
    await mkdir(path.join(dir, "src"), { recursive: true });
    await writeFile(
      path.join(dir, "tsconfig.json"),
      JSON.stringify(
        { extends: "../../packages/ui/tsconfig.native.json", include: ["src"] },
        null,
        2,
      ),
    );
    await writeFile(
      path.join(dir, "package.json"),
      JSON.stringify(
        {
          name: "schema-parity",
          private: true,
          zgui: {
            id: "dev.test.schema-parity",
            window: {
              title: "Parity",
              width: 640,
              height: 480,
              minWidth: 320,
              minHeight: 240,
              resizable: false,
            },
            capabilities: {
              shellSurfaces: true,
              controlChannel: true,
              singleInstance: true,
              clipboard: { read: true },
              sqlite: { databases: ["app_state"] },
              files: { directories: [{ path: "$XDG_DOCUMENTS_DIR/notes", mode: "readwrite" }] },
              systemFiles: { read: true, directories: [{ path: "/", mode: "read" }] },
              network: {
                originRules: [
                  { origin: "https://api.example.com" },
                  {
                    origin: "http://127.0.0.1:8080",
                    allowInsecureHttp: true,
                    allowPrivateNetworks: true,
                  },
                ],
              },
            },
          },
        },
        null,
        2,
      ),
    );
    await writeFile(
      path.join(dir, "src/main.tsx"),
      `import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <text>ok</text>, root);
}
`,
    );

    const { manifest } = await buildApplication("schema-parity", {
      root: workspaceRoot,
      appDirectory: dir,
    });
    validateAgainstSchema(manifest, schema, schema);
    assert.equal(manifest.capabilities?.singleInstance, true);

    assert.throws(
      () =>
        validateAgainstSchema(
          {
            ...manifest,
            capabilities: {
              files: { directories: [{ path: "$WAYLAND_DISPLAY/escape", mode: "read" }] },
            },
          },
          schema,
          schema,
        ),
      /failed pattern|did not match anyOf/,
    );
    assert.throws(
      () =>
        validateAgainstSchema(
          {
            ...manifest,
            capabilities: {
              network: { originRules: [{ origin: "https://api.example.com", extraFlag: true }] },
            },
          },
          schema,
          schema,
        ),
      /unexpected property extraFlag/,
    );
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
