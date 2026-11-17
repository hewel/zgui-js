export function createDelayed() {
  let timer: number | undefined, generation = 0;
  const cancel = () => {
    generation += 1;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  return { cancel, after(delay: number, action: () => void) {
    cancel();
    if (!Number.isFinite(delay) || delay < 0) throw new RangeError("Hover delay must be finite and non-negative");
    const current = generation;
    const run = () => { if (current !== generation) return; timer = undefined; action(); };
    if (delay === 0) run(); else timer = setTimeout(run, delay);
  } };
}
