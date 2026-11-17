/** Object styles use canonical CSS kebab-case names and string values, never DOM numeric coercion. */
export type NativeStyleName =
  | "display" | "position" | "inset" | "top" | "right" | "bottom" | "left" | "visibility"
  | "flex-direction" | "flex-wrap" | "flex-grow" | "flex-shrink" | "flex-basis"
  | "align-items" | "justify-content" | "gap" | "row-gap" | "column-gap"
  | "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
  | "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
  | "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height"
  | "color" | "background" | "background-color" | "border" | "border-color" | "border-width" | "border-radius"
  | "font-family" | "font-size" | "font-weight" | "line-height" | "opacity" | "overflow"
  | `--${string}`;
export type NativeStyle = string | { [Name in NativeStyleName]?: string | null };
