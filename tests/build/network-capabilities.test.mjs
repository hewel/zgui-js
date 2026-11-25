import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { bundle, publishApplication, workspaceRoot } from "../../packages/build/src/build.ts";

test("capabilities.network normalizes exact origins and rejects wildcards, paths, userinfo, insecure, and private defaults", async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  const dir = await mkdtemp(path.join(workspaceRoot, "target", "zgui-net-cap-"));
  try {
    await mkdir(path.join(dir, "src"), { recursive: true });
    await writeFile(
      path.join(dir, "package.json"),
      JSON.stringify({ name: "net-cap", private: true, zgui: { id: "dev.test.net-cap" } }, null, 2),
    );
    const entry = path.join(dir, "src/main.tsx");
    await writeFile(
      entry,
      `import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <text>ok</text>, root);
}
`,
    );

    const compiled = await bundle(entry, { root: workspaceRoot, appDirectory: dir });
    const publish = capabilities =>
      publishApplication("net-cap", compiled, {
        root: workspaceRoot,
        appDirectory: dir,
        extraCapabilities: capabilities,
      });

    const good = await publish({
      network: {
        origins: ["HTTPS://API.Example.COM:443", "wss://Stream.Example.com:8443"],
      },
    });
    assert.deepEqual(good.manifest.capabilities?.network, {
      origins: ["https://api.example.com", "wss://stream.example.com:8443"],
    });

    const badCases = [
      ["wildcard", { network: { origins: ["https://*.example.com"] } }, /invalid origin/i],
      ["path", { network: { origins: ["https://api.example.com/v1"] } }, /path/i],
      ["trailing-slash", { network: { origins: ["https://api.example.com/"] } }, /path/i],
      ["userinfo", { network: { origins: ["https://user:pass@api.example.com"] } }, /userinfo/i],
      ["insecure-http", { network: { origins: ["http://api.example.com"] } }, /allowInsecureHttp/i],
      ["insecure-ws", { network: { origins: ["ws://api.example.com"] } }, /allowInsecureHttp/i],
      [
        "private-loopback",
        { network: { origins: ["https://127.0.0.1:8443"] } },
        /allowPrivateNetworks/i,
      ],
      [
        "private-localhost",
        { network: { origins: ["https://localhost:8443"] } },
        /allowPrivateNetworks/i,
      ],
      [
        "private-rfc1918",
        { network: { origins: ["https://192.168.1.10"] } },
        /allowPrivateNetworks/i,
      ],
    ];

    for (const [label, caps, pattern] of badCases) {
      await assert.rejects(() => publish(caps), pattern, `expected ${label} to be rejected`);
    }

    const optIn = await publish({
      network: {
        origins: ["http://127.0.0.1:8080", "ws://127.0.0.1:8080"],
        allowInsecureHttp: true,
        allowPrivateNetworks: true,
      },
    });
    assert.deepEqual(optIn.manifest.capabilities?.network, {
      origins: ["http://127.0.0.1:8080", "ws://127.0.0.1:8080"],
      allowInsecureHttp: true,
      allowPrivateNetworks: true,
    });

    // Lossless per-origin rules keep strict origins isolated from insecure/private origins.
    const perOrigin = await publish({
      network: {
        origins: ["https://public.example.com"],
        originRules: [
          {
            origin: "HTTP://127.0.0.1:8080",
            allowInsecureHttp: true,
            allowPrivateNetworks: true,
          },
        ],
      },
    });
    assert.deepEqual(perOrigin.manifest.capabilities?.network, {
      originRules: [
        {
          origin: "http://127.0.0.1:8080",
          allowInsecureHttp: true,
          allowPrivateNetworks: true,
        },
        {
          origin: "https://public.example.com",
        },
      ],
    });

    await assert.rejects(
      () =>
        publish({
          network: {
            originRules: [{ origin: "http://public.example.com" }],
          },
        }),
      /allowInsecureHttp/i,
    );
    await assert.rejects(
      () =>
        publish({
          network: {
            originRules: [{ origin: "https://127.0.0.1:8443" }],
          },
        }),
      /allowPrivateNetworks/i,
    );
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
