import type { NativeNode } from "../types.js";
import { renderSlider } from "../primitives/slider.js";
import type { SliderPrimitiveProps } from "../primitives/slider.js";
import { append, classes, element, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/SliderStyle.js";

export interface SliderProps extends Omit<SliderPrimitiveProps, "children"> {
  children?: never;
}

export function Slider(props: SliderProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Slider", props);
  let bindTrack!: (node: NativeNode) => void;
  const root = renderSlider(props, ({ fraction, trackRef }) => {
    bindTrack = trackRef;
    return {
      get __zguiClass() {
        return classes(sheet.className, "zui-slider");
      },
      get __zguiStyle() {
        return { "--zui-slider-fraction": `${(fraction() * 100).toFixed(4)}%` };
      },
    };
  });
  const track = element("box", { class: "zui-slider__track", ariaHidden: true });
  bindTrack(track);
  append(track, element("box", { class: "zui-slider__range", ariaHidden: true }));
  const thumb = element("box", { class: "zui-slider__thumb", ariaHidden: true });
  append(root, track);
  append(root, thumb);
  return root;
}
