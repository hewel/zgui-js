import { createSignal } from "#solid";
const [revision, setRevision] = createSignal(0);
export function trackTreeOrder(): void { revision(); }
export function invalidateTreeOrder(): void { setRevision(value => value + 1); }
