import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";

function resolveRef(schema, root) {
  if (!schema.$ref) return schema;
  const target = schema.$ref.slice("#/$defs/".length);
  const resolved = root.$defs?.[target];
  assert.ok(resolved, `missing schema $def ${target}`);
  return resolved;
}

function validateAgainstSchema(value, rawSchema, root, ptr = "$") {
  const schema = resolveRef(rawSchema, root);
  if (schema.const !== undefined) {
    assert.equal(value, schema.const, `${ptr}: expected const ${JSON.stringify(schema.const)}`);
    return;
  }
  if (schema.enum !== undefined) {
    assert.ok(schema.enum.includes(value), `${ptr}: ${JSON.stringify(value)} not in enum`);
    return;
  }
  if (schema.anyOf) {
    const failures = [];
    for (const branch of schema.anyOf) {
      try {
        validateAgainstSchema(value, branch, root, ptr);
        return;
      } catch (error) {
        failures.push(error instanceof Error ? error.message : String(error));
      }
    }
    assert.fail(`${ptr}: did not match anyOf (${failures.join("; ")})`);
  }
  if (schema.type === "boolean") {
    assert.equal(typeof value, "boolean", `${ptr}: expected boolean`);
    return;
  }
  if (schema.type === "integer") {
    assert.ok(Number.isInteger(value), `${ptr}: expected integer`);
    if (schema.minimum !== undefined) assert.ok(value >= schema.minimum, `${ptr}: below minimum`);
    if (schema.maximum !== undefined) assert.ok(value <= schema.maximum, `${ptr}: above maximum`);
    return;
  }
  if (schema.type === "string" || typeof value === "string") {
    assert.equal(typeof value, "string", `${ptr}: expected string`);
    if (schema.minLength !== undefined)
      assert.ok(value.length >= schema.minLength, `${ptr}: too short`);
    if (schema.maxLength !== undefined)
      assert.ok(value.length <= schema.maxLength, `${ptr}: too long`);
    if (schema.pattern !== undefined) {
      assert.match(value, new RegExp(schema.pattern), `${ptr}: failed pattern ${schema.pattern}`);
    }
    if (!schema.anyOf) return;
  }
  if (schema.type === "array") {
    assert.ok(Array.isArray(value), `${ptr}: expected array`);
    if (schema.maxItems !== undefined)
      assert.ok(value.length <= schema.maxItems, `${ptr}: maxItems`);
    if (schema.uniqueItems) {
      const seen = new Set(value.map(v => JSON.stringify(v)));
      assert.equal(seen.size, value.length, `${ptr}: expected uniqueItems`);
    }
    if (schema.items) {
      value.forEach((item, idx) =>
        validateAgainstSchema(item, schema.items, root, `${ptr}[${idx}]`),
      );
    }
    return;
  }
  if (schema.type === "object") {
    assert.ok(
      value !== null && typeof value === "object" && !Array.isArray(value),
      `${ptr}: expected object`,
    );
    for (const req of schema.required ?? []) {
      assert.ok(req in value, `${ptr}: missing required property ${req}`);
    }
    for (const [key, val] of Object.entries(value)) {
      if (schema.propertyNames) {
        validateAgainstSchema(key, schema.propertyNames, root, `${ptr}.${key}(name)`);
      }
      if (schema.properties && key in schema.properties) {
        validateAgainstSchema(val, schema.properties[key], root, `${ptr}.${key}`);
      } else if (schema.additionalProperties === false) {
        assert.fail(`${ptr}: unexpected property ${key}`);
      } else if (schema.additionalProperties && typeof schema.additionalProperties === "object") {
        validateAgainstSchema(val, schema.additionalProperties, root, `${ptr}.${key}`);
      }
    }
  }
}

test("package.schema.json aligns with Rust and TS manifests including clipboard, sqlite, files, and network.originRules", async () => {
  const schema = JSON.parse(
    await readFile(path.join(workspaceRoot, "schemas/package.schema.json"), "utf8"),
  );
  const capProps = schema.properties.capabilities.properties;
  for (const key of ["clipboard", "sqlite", "files", "network"]) {
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
              clipboard: { read: true },
              sqlite: { databases: ["app_state"] },
              files: { directories: [{ path: "$XDG_DOCUMENTS_DIR/notes", mode: "readwrite" }] },
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
