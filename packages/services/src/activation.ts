import { getOwner, onCleanup } from "#solid";

export interface ActivationInfo {
  readonly args: readonly string[];
  readonly cwd: string | null;
}

interface HostActivation {
  readonly getInitialActivation: () => ActivationInfo;
  readonly onActivation: (handler: (info: ActivationInfo) => void) => () => void;
}

function hostActivation(): HostActivation {
  const runtime = globalThis as unknown as { readonly __zguiServices: HostActivation };
  return runtime.__zguiServices;
}

/** Read the cold launch arguments and working directory from an active session. */
export function getInitialActivation(): ActivationInfo {
  return hostActivation().getInitialActivation();
}

/** Subscribe to later invocations. Cleanup is automatic when called under a Solid owner. */
export function onActivation(handler: (info: ActivationInfo) => void): () => void {
  const unsubscribe = hostActivation().onActivation(handler);
  if (getOwner()) onCleanup(unsubscribe);
  return unsubscribe;
}
