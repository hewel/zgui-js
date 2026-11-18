import { splitProps } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { append, classes, element, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/ProgressStyle.js";

export interface ProgressProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  value?: number | null;
  max?: number;
  label?: string | null;
}

export function Progress(props: ProgressProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Progress", props);
  const [own, rest] = splitProps(props, ["value", "max", "label"]);
  const measured = memo(() => {
    const max = own.max ?? 100;
    if (!Number.isFinite(max) || max <= 0) {
      throw new TypeError("Progress max must be a positive finite number");
    }
    const value = own.value;
    if (value === null || value === undefined) return { max, value: null };
    if (!Number.isFinite(value)) throw new TypeError("Progress value must be a finite number");
    return { max, value: Math.min(Math.max(value, 0), max) };
  });
  const node = element("box", mergeProps(
    {
      role: "progressIndicator" as const,
      get ariaValueMin() { measured(); return 0; },
      get ariaValueMax() { return measured().max; },
      get ariaValueNow() { return measured().value; },
      get ariaLabel() { return own.label ?? null; },
      get "data-state"() { return measured().value === null ? "indeterminate" : "determinate"; },
      get __zguiStyle() {
        const { max, value } = measured();
        return { "--zui-progress-fraction": value === null ? null : `${(value / max * 100).toFixed(4)}%` };
      },
    },
    rest,
    { get __zguiClass() { return classes(sheet.className, "zui-progress"); } },
  ));
  append(node, element("box", { class: "zui-progress__fill" }));
  return node;
}
