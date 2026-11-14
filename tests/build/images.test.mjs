import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, realpath, rm } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

test("Image forwards reactive src replacements/clears and rejects lazy child spreads", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "tests/build/.image-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src"));
  await writeFile(path.join(directory, "package.json"), '{"type":"module"}');
  await writeFile(path.join(directory, "tsconfig.json"), JSON.stringify({
    compilerOptions: { target: "ES2022", module: "ESNext", moduleResolution: "Bundler", lib: ["ES2022"], types: [], strict: true, noEmit: true, jsx: "preserve", jsxImportSource: "@zgui/ui" }, include: ["src"],
  }));
  await writeFile(path.join(directory, "src/main.tsx"), `
    import { Image, createSignal } from "@zgui/ui";
    import type { NativeNode } from "@zgui/ui";
    import { mergeProps, render } from "@zgui/ui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) { return render(() => undefined, root); }
    export function probe(root: NativeNode) {
      let update: (src: string | null | undefined) => void = () => {};
      let children = () => {};
      let reads = 0;
      const dispose = render(() => {
        const [props, setProps] = createSignal<{ src?: string | null }>({ src: "assets/first.png" });
        Image(mergeProps(props));
        update = src => setProps({ src });
        children = () => setProps(Object.defineProperty({ src: "assets/forbidden.png" }, "children", {
          enumerable: true, get() { reads++; return "forbidden"; }
        }));
        return undefined;
      }, root);
      update("assets/second.png"); update(null); update("assets/third.png"); update(undefined);
      let rejected = false;
      try { children(); } catch (error) { rejected = error instanceof TypeError; }
      finally { dispose(); }
      return { rejected, reads };
    }
  `);
  const { dist } = await buildApplication("image-fixture", { appDirectory: directory });
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zgui");
  const writes = [];
  try {
    // Only SDK forwarding/shape guarding is modeled here. Native loading is tested in Rust.
    Object.defineProperty(globalThis, "__zgui", { configurable: true, value: {
      createElement: tag => { assert.equal(tag, "image"); return { tag }; },
      setProperty: (_node, name, value) => { writes.push([name, value]); },
    } });
    const module = await import(pathToFileURL(path.join(await realpath(dist), "app.mjs")));
    assert.deepEqual(module.probe({}), { rejected: true, reads: 0 });
    assert.deepEqual(writes.filter(([name]) => name === "src"), [
      ["src", "assets/first.png"], ["src", "assets/second.png"], ["src", null],
      ["src", "assets/third.png"], ["src", undefined],
    ]);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zgui", previous);
    else delete globalThis.__zgui;
  }
});
