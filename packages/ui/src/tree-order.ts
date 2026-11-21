import { createSignal } from "#solid";
let observed = false;
let counter = 0;
const [revision, setRevision] = createSignal(0, {
  ownedWrite: true,
  unobserved() { observed = false; },
});
export function trackTreeOrder(): void {
  observed = true;
  revision();
}
export function invalidateTreeOrder(): void {
  counter += 1;
  if (observed) setRevision(counter);
}
