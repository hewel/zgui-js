import type { HostRequest } from "./index.js";

/** Opaque token scoped to the application that persisted the grant. */
export type GrantToken = string;
export type GrantKind = "workspace-file" | "workspace-dir" | "system-dir";
export type GrantAccessMode = "read" | "readwrite";

/** Persisted authority metadata; inventory does not expose the underlying path. */
export interface GrantInfo {
  readonly token: GrantToken;
  readonly kind: GrantKind;
  readonly label: string;
  readonly mode: GrantAccessMode;
  readonly createdMs: number;
}

interface HostGrantBinding {
  readonly listGrants: () => HostRequest<readonly GrantInfo[]>;
  readonly revokeGrant: (token: GrantToken) => HostRequest<boolean>;
  readonly clearGrants: () => HostRequest<number>;
}
// The native host owns this binding; read the service lazily at call time.
const runtime = globalThis as typeof globalThis & { readonly __zguiServices: HostGrantBinding };

/** List this application's persisted grants, including those it can no longer restore. */
export function listGrants(): HostRequest<readonly GrantInfo[]> {
  return runtime.__zguiServices.listGrants();
}

/** Revoke a persisted grant; existing live handles are not closed. */
export function revokeGrant(token: GrantToken): HostRequest<boolean> {
  return runtime.__zguiServices.revokeGrant(token);
}

/** Remove all persisted grants for this app and return the count. Live handles stay open. */
export function clearGrants(): HostRequest<number> {
  return runtime.__zguiServices.clearGrants();
}
