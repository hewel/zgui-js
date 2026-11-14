/** Host-owned registrations, not browser or Node compatibility. */
export {};

declare global {
  const console: {
    log(...values: unknown[]): void;
    info(...values: unknown[]): void;
    warn(...values: unknown[]): void;
    error(...values: unknown[]): void;
    debug(...values: unknown[]): void;
  };
  function setTimeout(callback: () => void, delay?: number): number;
  function clearTimeout(handle: number): void;
  function requestAnimationFrame(callback: (timestamp: number) => void): number;
  function cancelAnimationFrame(handle: number): void;
  function queueMicrotask(callback: () => void): void;
}
