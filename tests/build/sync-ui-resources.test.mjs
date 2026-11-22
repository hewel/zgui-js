import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { workspaceRoot } from "../../packages/build/src/build.ts";
import { adaptUpstreamCss, syncUiResources } from "../../packages/build/src/sync-ui-resources.ts";
import { sheet as committedTextarea } from "../../packages/ui/src/components/generated/TextareaStyle.ts";

test("adaptUpstreamCss reproduces the committed TextareaStyle overflow-y: auto adaptation", () => {
  const upstreamCss = committedTextarea.css.replace(
    "overflow-x: hidden;\n        overflow-y: auto;",
    "overflow: hidden;",
  );
  assert.notEqual(upstreamCss, committedTextarea.css);
  const adapted = adaptUpstreamCss(
    committedTextarea.name,
    committedTextarea.className,
    upstreamCss,
  );
  assert.equal(adapted, committedTextarea.css);
  assert.match(adapted, /overflow-x: hidden;/);
  assert.match(adapted, /overflow-y: auto;/);
});

test("syncUiResources writes adapted styles into an explicit --out directory without mutating the repo", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-sync-resources-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const snapshotPath = path.join(dir, "snapshot.json");
  const outDir = path.join(dir, "generated");
  const upstreamTextareaCss = committedTextarea.css.replace(
    "overflow-x: hidden;\n        overflow-y: auto;",
    "overflow: hidden;",
  );
  await writeFile(
    snapshotPath,
    JSON.stringify({
      revision: "80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a",
      styles: {
        TextareaStyle: {
          className: committedTextarea.className,
          css: upstreamTextareaCss,
        },
        CollapsibleStyle: {
          className: "zui-collapsible",
          css: ".zui-collapsible { display: block; }",
        },
      },
      themes: {
        default: { light: ":root { --zui-test: 1; }", dark: ":root { --zui-test: 2; }" },
      },
      tokens: ["--zui-test"],
      icons: {
        CHECK: { name: "check", viewBox: 24, d: "M20 6 9 17l-5-5" },
      },
    }),
  );

  const count = await syncUiResources(snapshotPath, outDir);
  assert.equal(count, 4);
  const regeneratedTextarea = await readFile(path.join(outDir, "TextareaStyle.ts"), "utf8");
  const committedFile = await readFile(
    path.join(workspaceRoot, "packages/ui/src/components/generated/TextareaStyle.ts"),
    "utf8",
  );
  assert.equal(regeneratedTextarea, committedFile);
  const collapsible = await readFile(path.join(outDir, "CollapsibleStyle.ts"), "utf8");
  assert.match(
    collapsible,
    /\.zui-collapsible__content\[data-collapsed\] \{ visibility: hidden; \}/,
  );
});
