import assert from "node:assert/strict";
import { runSmoke } from "../../../target/solid-smoke.mjs";

assert.equal(runSmoke(), "solid-smoke:ok");
assert.equal(runSmoke(), "solid-smoke:ok");
console.log("solid-smoke:ok (two synchronous runs, cleanup verified)");
