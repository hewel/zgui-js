import {
  render, effect, memo, createComponent, createElement, createTextNode,
  insertNode, insert, spread, setProp, mergeProps, use,
} from "@zgui/ui/renderer";
import "@zgui/ui/runtime-globals";
import type { NativeChild, NativeNode } from "@zgui/ui";
import type { JSX } from "@zgui/ui/jsx-runtime";

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
  effect(previous => (previous ?? 0) + 1, 0);
  const value = memo(() => "memo");
  const child: NativeChild = createComponent(props => props.text, { text: value() });
  const element: JSX.Element = child;
  use((borrowed, argument: string) => { void borrowed; return argument; }, node, last);
  void [element, first];
  return render(() => [node, child, true], root);
}
