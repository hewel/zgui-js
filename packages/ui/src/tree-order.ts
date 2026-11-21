import { createSignal } from "#solid";
let observed = false;
let dirty = false;
let counter = 0;
const [revision, setRevision] = createSignal(0, {
  ownedWrite: true,
  unobserved() {
    observed = false;
    dirty = false;
  },
});
export function trackTreeOrder(): void {
  observed = true;
  dirty = false;
  revision();
}
export function invalidateTreeOrder(): void {
  if (!observed || dirty) return;
  dirty = true;
  counter += 1;
  setRevision(counter);
}
