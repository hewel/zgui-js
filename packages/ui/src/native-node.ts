declare const nativeNode: unique symbol;

/**
 * Borrowed opaque handle, never a native numeric ID. Permanently removed branches
 * invalidate their handles at the end of the synchronous host turn. A retained
 * ref does not retain native ownership; cross-turn detached-node reuse is unsupported.
 */
export interface NativeNode {
  readonly [nativeNode]: true;
}
