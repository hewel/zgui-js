import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { bundle } from "../../packages/build/src/build.mjs";

export function createHost() {
  let nextId = 0;
  const nodes = new Map();
  const parents = new Map();
  const stylesheets = new Map();
  const installs = [];
  const focused = { current: null };
  const rects = new Map();
  const textContent = node => node.tag === "#text" ? node.props.get("text") ?? "" : node.children.map(textContent).join("");
  const makeNode = tag => {
    const node = { tag, id: ++nextId, props: new Map(), callbacks: {}, children: [] };
    nodes.set(node.id, node);
    return node;
  };
  const host = {
    createElement: makeNode,
    createTextNode: text => {
      if (typeof text !== "string") throw new TypeError("createTextNode expects a string");
      const node = makeNode("#text");
      node.props.set("text", text);
      return node;
    },
    replaceText: (node, text) => {
      if (typeof text !== "string") throw new TypeError("replaceText expects a string");
      node.props.set("text", text);
    },
    isTextNode: node => node.tag === "#text",
    insertNode: (parent, node, anchor) => {
      if (node === anchor) return;
      const previous = parents.get(node);
      if (previous) {
        const index = previous.children.indexOf(node);
        if (index >= 0) previous.children.splice(index, 1);
      }
      parents.set(node, parent);
      if (anchor !== undefined && anchor !== null) {
        const index = parent.children.indexOf(anchor);
        if (index < 0) throw new Error("anchor is not a child of the parent");
        parent.children.splice(index, 0, node);
      } else {
        parent.children.push(node);
      }
    },
    removeNode: (parent, node) => {
      const index = parent.children.indexOf(node);
      if (index >= 0) parent.children.splice(index, 1);
      parents.delete(node);
    },
    getParentNode: node => parents.get(node) ?? null,
    getFirstChild: node => node.children[0] ?? null,
    getNextSibling: node => {
      const parent = parents.get(node);
      if (!parent) return null;
      const index = parent.children.indexOf(node);
      return index >= 0 ? parent.children[index + 1] ?? null : null;
    },
    setProperty: (node, name, value) => {
      if (typeof value === "function") {
        node.callbacks[name] = value;
        return;
      }
      delete node.callbacks[name];
      if (value === undefined || value === null) node.props.delete(name);
      else node.props.set(name, value);
    },
    installStylesheet: (name, css) => {
      if (stylesheets.get(name) === css) return;
      installs.push(name);
      stylesheets.set(name, css);
    },
    removeStylesheet: name => { stylesheets.delete(name); },
    focus: (node, reveal = false) => { focused.current = node; focused.reveal = reveal; },
    getWindowRect: node => rects.get(node) ?? null,
    getTextContent: textContent,
    precedes: (first, second) => {
      if (first === second) return false;
      const rootOf = node => {
        let current = node;
        while (parents.get(current)) current = parents.get(current);
        return current;
      };
      if (rootOf(first) !== rootOf(second)) return false;
      const order = new Map();
      let index = 0;
      const walk = node => {
        order.set(node, index++);
        for (const child of node.children) walk(child);
      };
      walk(rootOf(first));
      return order.get(first) < order.get(second);
    },
    clearRoot: root => {
      for (const child of root.children) parents.delete(child);
      root.children.length = 0;
    },
  };
  const attached = node => {
    let current = node;
    while (current) {
      if (current.tag === "root") return true;
      current = parents.get(current);
    }
    return false;
  };
  const byTestId = testId =>
    [...nodes.values()].find(node => node.props.get("testId") === testId && attached(node));
  return { host, nodes, parents, stylesheets, installs, makeNode, byTestId, focused, rects };
}

export async function moduleFor(t, source) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-components-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const entry = path.join(directory, "main.tsx");
  await writeFile(entry, source);
  const result = await bundle(entry, { appDirectory: directory, hostEntry: false });
  for (const [name, bytes] of result.files) await writeFile(path.join(directory, name), bytes);
  return import(pathToFileURL(path.join(directory, "app.mjs")).href);
}
