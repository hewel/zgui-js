import {
  render, effect, memo, createComponent, createElement, createTextNode,
  insertNode, insert, spread, setProp, mergeProps, ref, applyRef,
} from "@hewel/zgui/renderer";
import "@hewel/zgui/runtime-globals";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import type { JSX } from "@hewel/zgui/jsx-runtime";

export function helpers(root: NativeNode): () => void {
  const node = createElement("column");
  const text = createTextNode("");
  insertNode(node, text, null);
  insert(node, () => ["text", 7, false, undefined]);
  spread(node, { children: "spread", ref: borrowed => { void borrowed; } });
  setProp(node, "children", "native", undefined);
  const properties = mergeProps({ value: 1, first: true }, { value: "last" });
  const last: string = properties.value;
  const first: boolean = properties.first;
  effect<number>(previous => (previous ?? 0) + 1, value => { void value; });
  const value = memo(() => "memo");
  const child: NativeChild = createComponent(props => props.text, { text: value() });
  const element: JSX.Element = child;
  ref(() => borrowed => { void borrowed; }, node);
  applyRef(borrowed => { void borrowed; }, node);
  void [element, first, last];
  return render(() => [node, child, true], root);
}
