import { createElement, effect, spread } from "./renderer/index.js";
import type { ImageProps, NativeNode } from "./types.js";

/** Native replaced content; source writes go through the native attribute hook. */
export function Image(props: ImageProps = {}): NativeNode {
  effect(() => "children" in props, hasChildren => {
    if (hasChildren) throw new TypeError("Image does not accept children");
  });
  const image = createElement("image");
  spread(image, props, true);
  return image;
}
