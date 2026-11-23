import { createSignal } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { Accessor, NativeNode } from "../types.js";
import { SliderPrimitive } from "../primitives/slider.js";
import type { SliderPrimitiveProps, SliderPrimitiveRenderState } from "../primitives/slider.js";
import { append, classes, element, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/SliderStyle.js";

export interface SliderProps extends Omit<SliderPrimitiveProps, "children"> {
  children?: never;
}

export function Slider(props: SliderProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Slider", props);
  const [fractionSource, setFractionSource] = createSignal<{ readonly get: Accessor<number> }>({
    get: () => 0,
  });
  return SliderPrimitive(
    mergeProps(props, {
      get __zguiClass() {
        return classes(sheet.className, "zui-slider");
      },
      get __zguiStyle() {
        return { "--zui-slider-fraction": `${(fractionSource().get() * 100).toFixed(4)}%` };
      },
      children: ({ fraction, trackRef }: SliderPrimitiveRenderState) => {
        setFractionSource({ get: fraction });
        const track = element("box", { class: "zui-slider__track", ariaHidden: true });
        trackRef(track);
        append(track, element("box", { class: "zui-slider__range", ariaHidden: true }));
        const thumb = element("box", { class: "zui-slider__thumb", ariaHidden: true });
        return [track, thumb];
      },
    }),
  );
}
