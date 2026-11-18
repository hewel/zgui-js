/** Native CSS names and their camelCase aliases; values are validated atomically by the host. */
type OrdinaryStyleName =
  | "display" | "position" | "inset" | "top" | "right" | "bottom" | "left" | "visibility"
  | "flex-direction" | "flex-wrap" | "flex-grow" | "flex-shrink" | "flex-basis"
  | "align-items" | "justify-content" | "gap" | "row-gap" | "column-gap"
  | "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
  | "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
  | "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height"
  | "color" | "background" | "background-color" | "border" | "border-color" | "border-width" | "border-radius"
  | "font-family" | "font-size" | "font-weight" | "line-height" | "opacity" | "overflow"
  | "z-index" | "order" | "transform" | "transform-origin" | "animation" | "animation-name"
  | "animation-duration" | "animation-delay" | "animation-timing-function" | "animation-iteration-count"
  | "animation-fill-mode" | "animation-direction" | "animation-play-state"
  | "transition" | "transition-property" | "transition-duration" | "transition-delay" | "transition-timing-function";
export type NativeStyleName = OrdinaryStyleName | `--${string}`;
type CamelCase<Name extends string> = Name extends `${infer Head}-${infer Tail}`
  ? `${Head}${Capitalize<CamelCase<Tail>>}` : Name;
type Unitless = "opacity" | "flex-grow" | "flex-shrink" | "font-weight" | "line-height"
  | "z-index" | "order" | "animation-iteration-count";
export type NativeStyle = string
  | { [Name in Exclude<OrdinaryStyleName | CamelCase<OrdinaryStyleName>, Unitless | CamelCase<Unitless>>]?: string | null | undefined }
    & { [Name in Unitless | CamelCase<Unitless>]?: string | number | null | undefined }
    & { [Name: `--${string}`]: string | number | null | undefined };
