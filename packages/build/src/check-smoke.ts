import assert from "node:assert/strict";

// The generated bundle does not exist when the build tools are typechecked.
const artifact = new URL("../../../target/solid-smoke.mjs", import.meta.url);
const { runSmoke }: { runSmoke: () => unknown } = await import(artifact.href);
assert.equal(runSmoke(), "solid-smoke:ok");
assert.equal(runSmoke(), "solid-smoke:ok");
console.log("solid-smoke:ok (two synchronous runs, cleanup verified)");
