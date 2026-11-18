import type { NativeStyle } from "./styles.js";

/** SDK implementation channel; never re-exported from the application package. */
export interface InternalProps {
  __zguiClass?: string;
  __zguiStyle?: Exclude<NativeStyle, string>;
  __zguiInvariantStyle?: Exclude<NativeStyle, string>;
}
