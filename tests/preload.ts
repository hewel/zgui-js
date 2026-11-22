// QuickJS-NG and Node.js do not expose browser `globalThis.reportError`.
// Solid 2's scheduler calls it on uncaught reactive flushes before rethrowing,
// which triggers Bun's unhandled-error hook even when a test expects the throw.
Object.defineProperty(globalThis, "reportError", {
  value: undefined,
  writable: true,
  configurable: true,
});

// Solid 2 logs full `[REACTIVITY_HALTED]` stack traces via `console.error`
// even when tests assert the thrown error, bloating `bun test` logs past 250 KiB.
if (process.env.ZGUI_VERBOSE_TEST_ERRORS !== "1") {
  const originalConsoleError = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    if (typeof args[0] === "string" && args[0].includes("[REACTIVITY_HALTED]")) {
      return;
    }
    originalConsoleError(...args);
  };
}
