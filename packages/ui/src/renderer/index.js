import { createRenderer } from "solid-js/universal";

// Resolve the bridge at invocation, so module preflight never allocates native nodes.
const renderer = createRenderer({
  createElement: tag => globalThis.__zgui.createElement(tag),
  createTextNode: text => globalThis.__zgui.createTextNode(text),
  replaceText: (node, text) => globalThis.__zgui.replaceText(node, text),
  isTextNode: node => globalThis.__zgui.isTextNode(node),
  insertNode: (parent, node, anchor) => globalThis.__zgui.insertNode(parent, node, anchor),
  removeNode: (parent, node) => globalThis.__zgui.removeNode(parent, node),
  getParentNode: node => globalThis.__zgui.getParentNode(node),
  getFirstChild: node => globalThis.__zgui.getFirstChild(node),
  getNextSibling: node => globalThis.__zgui.getNextSibling(node),
  setProperty: (node, name, value, previous) =>
    globalThis.__zgui.setProperty(node, name, value, previous),
});

export const {
  render, effect, memo, createComponent, createElement, createTextNode,
  insertNode, insert, spread, setProp, mergeProps, use,
} = renderer;
