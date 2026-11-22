import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";

const PRIMITIVES_APP = `
import { createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import {
  MenuCheckboxItemPrimitive,
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRadioGroupPrimitive,
  MenuRadioItemPrimitive,
  MenuRootPrimitive,
  MenuSubPrimitive,
  createDelayed,
  createDismissLayer,
  createOverlayState,
  createPresence,
  createRetainedPresence,
  createRovingFocus,
} from "@hewel/zgui/primitives";

export const hostApiVersion = 1;

function Demo() {
  const [checked, setChecked] = createSignal(false);
  const [choice, setChoice] = createSignal("a");
  const overlay = createOverlayState({ defaultOpen: true });
  const presence = createPresence(overlay.open);
  const retained = createRetainedPresence(() => (overlay.open() ? choice() : null), {
    settled: undefined,
  });
  const delayed = createDelayed();
  const roving = createRovingFocus(() => "vertical");
  void [presence, retained, delayed, roving, createDismissLayer];

  return (
    <MenuRootPrimitive open={overlay.open()} onOpenChange={overlay.setOpen}>
      <MenuListPrimitive label="Primitives Menu">
        <MenuItemPrimitive onSelect={() => overlay.setOpen(false)}>Action</MenuItemPrimitive>
        <MenuCheckboxItemPrimitive checked={checked()} onCheckedChange={setChecked}>
          Check
        </MenuCheckboxItemPrimitive>
        <MenuRadioGroupPrimitive value={choice()} onValueChange={setChoice}>
          <MenuRadioItemPrimitive value="a">Alpha</MenuRadioItemPrimitive>
          <MenuRadioItemPrimitive value="b">Beta</MenuRadioItemPrimitive>
        </MenuRadioGroupPrimitive>
        <MenuSubPrimitive defaultOpen={false}>
          <MenuItemPrimitive>Sub</MenuItemPrimitive>
        </MenuSubPrimitive>
      </MenuListPrimitive>
    </MenuRootPrimitive>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Demo />, root);
}
`;

test("@hewel/zgui/primitives compiles with zero generated/*Style imports, zero components.css, and exactOptionalPropertyTypes", async t => {
  const dir = await mkdtemp(path.join(workspaceRoot, "tests/build/.fixture-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  await mkdir(path.join(dir, "src"), { recursive: true });
  await writeFile(path.join(dir, "package.json"), '{"type":"module"}\n');
  await writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
      compilerOptions: {
        exactOptionalPropertyTypes: true,
      },
      include: ["src/**/*.ts", "src/**/*.tsx"],
    }),
  );
  await writeFile(path.join(dir, "src/main.tsx"), PRIMITIVES_APP);

  const { dist, manifest } = await buildApplication("primitives-check", { appDirectory: dir });
  assert.deepEqual(manifest.styles, ["app.css"], "primitives bundle must not emit components.css");
  const info = JSON.parse(await readFile(path.join(dist, "build-info.json"), "utf8"));
  const styleModules = info.graph.modules.filter(mod => /components\/generated\/.*Style/.test(mod));
  assert.deepEqual(
    styleModules,
    [],
    `primitives bundle must not import generated stylesheets: ${styleModules.join(", ")}`,
  );
});
