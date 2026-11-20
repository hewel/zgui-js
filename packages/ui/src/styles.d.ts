/** Native CSS names and their camelCase aliases; values are validated atomically by the host. */
type OrdinaryStyleName =
  | "display" | "position" | "inset" | "top" | "right" | "bottom" | "left" | "visibility" | "box-sizing"
  | "flex" | "flex-direction" | "flex-wrap" | "flex-grow" | "flex-shrink" | "flex-basis"
  | "align-items" | "align-self" | "justify-content" | "justify-self" | "gap" | "row-gap" | "column-gap"
  | "grid-template-columns" | "grid-template-rows" | "grid-column" | "grid-row" | "grid-area" | "grid-auto-flow"
  | "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
  | "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
  | "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height"
  | "object-fit" | "object-position"
  | "color" | "background" | "background-color"
  | "border" | "border-top" | "border-right" | "border-bottom" | "border-left"
  | "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style"
  | "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width"
  | "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color"
  | "border-color" | "border-style" | "border-width" | "border-radius"
  | "outline" | "outline-color" | "outline-style" | "outline-width" | "outline-offset"
  | "font-family" | "font-size" | "font-weight" | "line-height" | "letter-spacing"
  | "font-variant-numeric" | "font-feature-settings" | "font-variation-settings" | "font-style" | "font-stretch"
  | "text-align" | "text-transform" | "text-decoration" | "text-decoration-line" | "white-space" | "text-overflow"
  | "opacity" | "overflow" | "overflow-x" | "overflow-y"
  | "box-shadow" | "text-shadow" | "filter" | "backdrop-filter" | "mix-blend-mode" | "isolation" | "clip-path"
  | "pointer-events" | "user-select" | "translate" | "rotate" | "scale"
  | "z-index" | "order" | "transform" | "transform-origin" | "animation" | "animation-name"
  | "animation-duration" | "animation-delay" | "animation-timing-function" | "animation-iteration-count"
  | "animation-fill-mode" | "animation-direction" | "animation-play-state"
  | "transition" | "transition-property" | "transition-duration" | "transition-delay" | "transition-timing-function";
export type NativeStyleName = OrdinaryStyleName | `--${string}`;
type CamelCase<Name extends string> = Name extends `${infer Head}-${infer Tail}`
  ? `${Head}${Capitalize<CamelCase<Tail>>}` : Name;
type Unitless = "opacity" | "flex" | "flex-grow" | "flex-shrink" | "font-weight" | "line-height"
  | "z-index" | "order" | "animation-iteration-count" | "scale";
export type NativeStyle = string
  | { [Name in Exclude<OrdinaryStyleName | CamelCase<OrdinaryStyleName>, Unitless | CamelCase<Unitless>>]?: string | null | undefined }
    & { [Name in Unitless | CamelCase<Unitless>]?: string | number | null | undefined }
    & { [Name: `--${string}`]: string | number | null | undefined };
