// QuickJS-NG and Node.js do not expose browser `globalThis.reportError`.
// Solid 2's scheduler calls it on uncaught reactive flushes before rethrowing,
// which triggers Bun's unhandled-error hook even when a test expects the throw.
Object.defineProperty(globalThis, "reportError", {
  value: undefined,
  writable: true,
  configurable: true,
});
