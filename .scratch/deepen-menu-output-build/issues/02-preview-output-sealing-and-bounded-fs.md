# 02: Unify preview Output sealing, controller finalization, and bounded file reads

**What to build:** Replace untyped `Output` inventory values with strongly typed **Artifact inventory** records, unify `doctor`, `run`, and `compare` worker sealing and controller finalization (retiring `OutputState`/`adopt`, fixing unsealed `--out` on `doctor` and `compare`, aligning `compare` `8 KiB` stdout overflow to `E_OUTPUT_LIMIT` exit `8`, and preserving `originalError`), and centralize all bounded file reads in `crates/native-host/src/bounded_fs.rs`.

**Blocked by:** none

**Status:** resolved

- [x] `crates/native-host/src/preview/output.rs` models **Artifact inventory** entries with typed Rust structs/enums (`complete`, `historical`, `partial`), preserves exact `artifacts.json` / `worker-artifacts.json` JSON output, and retires `OutputState`, `Output::into_state`, `Output::adopt`, and `supervisor::Report.output_state`.
- [x] `doctor_worker` seals `--out` via `Output::seal_worker()`, and `doctor`, `run`, and `compare` share unified controller finalization: `restore_worker()` / failure `recover()`, `8 KiB` stdout limit enforcement via `E_OUTPUT_LIMIT` (exit `8`, replacing `compare.rs`'s `E_INTERNAL` exit `9`), earliest `originalError` preservation across both stdout-bound and `finish` errors, and `artifacts.json` + `result.json` sealing when `--out` is present.
- [x] New `crates/native-host/src/bounded_fs.rs` exposes two explicit contracts: `read_contained(root, relative, limit)` (canonical relative path validation, component-wise symlink rejection, `O_NOFOLLOW | O_NONBLOCK` on Unix, post-open `fstat` regular-file/size check, and `take(limit + 1)` growth guard) and `read_input(path, limit)` (allows top-level CLI symlinks while opening with `O_NONBLOCK` on Unix, verifying `fstat` regular-file + byte cap, and guarding growth via `take(limit + 1)`).
- [x] `package.rs`, `shell.rs`, `preview/output.rs`, `preview/saved.rs`, `preview/evidence.rs`, `preview/profile.rs`, `preview/fixture.rs`, `preview/scenario.rs`, and `preview/compare.rs` delegate their bounded regular-file reads to `bounded_fs`, mapping `BoundedReadError` back to each caller's existing protocol error code (`E_BUNDLE`, `E_SCHEMA`, `E_FONT`, `E_OUTPUT_IO`, `E_OUTPUT_LIMIT`).
- [x] New Rust unit tests cover `bounded_fs` (`read_contained` symlink escape rejection, `read_input` permitted top-level symlink vs non-blocking Unix FIFO rejection, byte-limit overflow), `doctor --out` worker sealing and overflow sealing with `originalError`, and `compare` `E_OUTPUT_LIMIT` (exit `8`) + `originalError` preservation; `pnpm test:rust` (including `clippy --all-targets -D warnings`) and preview `doctor` / `run` / `compare` / `inspect` tests pass after the final edit.

## Verification

- Running `tests/preview/comparison.test.mjs` and `tests/preview/ci.test.mjs` exposed a stale `preview/baselines/counter/run/.preview-owner` dotfile (`2141424\n`) omitted during `067d80b`'s baseline refresh (`artifacts.json` recorded `sha256:28c1e6a01f0b105135c51e30aac583f26da6e3281424fe1d18df89b64363a493`, corresponding to `2861801\n`); restored `.preview-owner` to `2861801\n`.
