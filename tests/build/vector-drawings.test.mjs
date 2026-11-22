import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

async function setup(t, source) {
  const mod = await moduleFor(t, source);
  const harness = createHost();
  const root = harness.makeNode("root");
  globalThis.__zgui = harness.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  return {
    mod,
    harness,
    root,
    mount: (...args) => {
      dispose = harness.runTurn(() => mod.mount(root, ...args));
    },
    unmount: () => {
      const cleanup = dispose;
      dispose = undefined;
      harness.runTurn(cleanup);
    },
  };
}

test("vector forwards reactive SVG documents and clears them without losing path data", async t => {
  const { mod, harness, root, mount } = await setup(
    t,
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    const [svg, setSvg] = createSignal<string | null>();
    export { setSvg };
    export function mount(root: NativeNode) {
      return renderApp(() => <vector svg={svg()} d="M0 0 L8 8" viewBox="0 0 8 8" />, root);
    }
  `,
  );
  const first = '<svg viewBox="0 0 8 8"><rect width="8" height="8" fill="#f00"/></svg>';
  const second = '<svg viewBox="0 0 8 8"><circle cx="4" cy="4" r="3" stroke="#00f"/></svg>';
  mod.setSvg(first);
  mount();
  const [vector] = root.children;
  assert.equal(vector.props.get("svg"), first);

  harness.runTurn(() => mod.setSvg(second));
  assert.equal(vector.props.get("svg"), second);
  harness.runTurn(() => mod.setSvg(null));
  assert.equal(vector.props.has("svg"), false);
  harness.runTurn(() => mod.setSvg(first));
  assert.equal(vector.props.get("svg"), first);
  harness.runTurn(() => mod.setSvg(undefined));
  assert.equal(vector.props.has("svg"), false);
  assert.equal(vector.props.get("d"), "M0 0 L8 8");
  assert.equal(vector.props.get("viewBox"), "0 0 8 8");
});

test("an unowned canvas scene updates and binds to multiple vectors until explicitly disposed", async t => {
  const { mod, harness, root, mount, unmount } = await setup(
    t,
    `
    import { createCanvasScene, createSignal, renderApp } from "@hewel/zgui";
    import type { CanvasScene, NativeNode, Setter } from "@hewel/zgui";
    export { createCanvasScene };
    export function mount(root: NativeNode, scene: CanvasScene) {
      return renderApp(() => {
        const [binding, setBinding] = createSignal<CanvasScene | null | undefined>(scene);
        setCanvas = setBinding;
        return <column>
          <vector canvas={binding()} svg='<svg viewBox="0 0 8 8"/>' d="M0 0 L8 8" />
          <vector canvas={scene} d="M1 1 L7 7" />
        </column>;
      }, root);
    }
    export let setCanvas: Setter<CanvasScene | null | undefined>;
  `,
  );
  const scene = mod.createCanvasScene();
  const [record] = harness.canvasScenes;
  assert.equal(typeof scene.token, "number");
  assert.equal(scene.token, record.token);
  const commands = [{ kind: "circle", x: 4, y: 4, radius: 3, stroke: { color: "#abc", width: 1 } }];
  const transform = [1, 0, 0, 1, 5, 6];
  scene.update(commands);
  mount(scene);

  const [first, second] = root.children[0].children;
  assert.equal(first.props.get("canvas"), scene.token);
  assert.equal(second.props.get("canvas"), scene.token);
  assert.deepEqual(record.commands, commands);
  harness.runTurn(() => scene.update(null, transform));
  assert.deepEqual(record.commands, commands, "a transform-only update retains geometry");
  assert.deepEqual(record.transform, transform);
  harness.runTurn(() => scene.update([]));
  assert.deepEqual(record.commands, []);
  assert.deepEqual(record.transform, transform);

  harness.runTurn(() => mod.setCanvas(null));
  assert.equal(first.props.has("canvas"), false);
  assert.equal(second.props.get("canvas"), scene.token);
  harness.runTurn(() => mod.setCanvas(scene));
  assert.equal(first.props.get("canvas"), scene.token);
  harness.runTurn(() => mod.setCanvas(undefined));
  assert.equal(first.props.has("canvas"), false);
  harness.runTurn(() => mod.setCanvas(scene));
  assert.equal(first.props.get("canvas"), scene.token);

  harness.runTurn(() => scene.dispose());
  assert.equal(first.props.has("canvas"), false);
  assert.equal(second.props.has("canvas"), false);
  assert.equal(first.props.get("svg"), '<svg viewBox="0 0 8 8"/>');
  assert.equal(first.props.get("d"), "M0 0 L8 8");
  assert.equal(second.props.get("d"), "M1 1 L7 7");
  scene.dispose();
  assert.equal(record.disposals, 1);
  assert.throws(() => scene.update(null));

  const unowned = mod.createCanvasScene();
  harness.runTurn(() => mod.setCanvas(unowned));
  assert.equal(first.props.get("canvas"), unowned.token);
  unmount();
  unowned.update(commands);
  unowned.dispose();
});

test("Solid disposes a canvas scene when its owning branch is removed", async t => {
  const { mod, harness, root, mount, unmount } = await setup(
    t,
    `
    import { Show, createCanvasScene, createSignal, renderApp } from "@hewel/zgui";
    import type { CanvasScene, NativeNode } from "@hewel/zgui";
    const [visible, setVisible] = createSignal(true);
    export { setVisible };
    export let scene: CanvasScene;
    function Drawing() {
      scene = createCanvasScene();
      return <vector canvas={scene} />;
    }
    export function mount(root: NativeNode) {
      return renderApp(() => <Show when={visible()}><Drawing /></Show>, root);
    }
  `,
  );
  mount();
  const first = mod.scene;
  assert.equal(root.children[0].props.get("canvas"), first.token);
  harness.runTurn(() => mod.setVisible(false));
  assert.equal(root.children.length, 0);
  assert.equal(harness.canvasScenes[0].disposals, 1);
  assert.throws(() => first.update(null));

  harness.runTurn(() => mod.setVisible(true));
  const second = mod.scene;
  assert.notEqual(second.token, first.token);
  assert.equal(root.children[0].props.get("canvas"), second.token);
  unmount();
  assert.equal(harness.canvasScenes[1].disposals, 1);
  assert.throws(() => second.update(null));
});
