# Third-Party Notices

`zgui-js` is licensed under the **Apache License, Version 2.0** (see [`LICENSE`](LICENSE)).
This document summarizes third-party and upstream components included in `zgui-js` npm packages,
prebuilt `zgui-host` binaries, and packaged desktop applications.

## 1. Upstream `zgui` (`zortax/zgui`)

- **Source:** https://github.com/zortax/zgui (pinned revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba`)
- **License:** Apache License, Version 2.0
- **Scope:** Vendored native crates under `vendor/` (linked into `zgui-host`) and adapted UI
  stylesheet/theme/icon resources under `@hewel/zgui` (`packages/ui/upstream/{LICENSE,NOTICE,ORIGIN}`).
- **Additional attributions:** See [`NOTICE`](NOTICE) for derived portions from GPUI (Apache-2.0),
  Windows Terminal (MIT), Linux kernel UAPI headers (MIT and GPL-2.0 WITH Linux-syscall-note), and
  `hyprland-protocols` (BSD-3-Clause).

## 2. Mozilla Public License 2.0 (`MPL-2.0`) — File-Scoped Source Notice

- **Component:** `vendor/zgui-style/src/driver/recalc.rs` (derived from Servo's `stylo` style
  engine, https://github.com/servo/stylo)
- **License:** Mozilla Public License, Version 2.0 (https://mozilla.org/MPL/2.0/)
- **Scope:** Applies solely to `vendor/zgui-style/src/driver/recalc.rs`, which is compiled into
  `zgui-host` binaries distributed in `@hewel/zgui-host-*` and packaged applications.
- **Source Code Availability (MPL-2.0 Section 3.2):** Complete corresponding source code for this
  file, including all `zgui-js` modifications, is available in the `zgui-js` repository at
  https://github.com/hewel/zgui-js (path `vendor/zgui-style/src/driver/recalc.rs`) and upstream at
  https://github.com/zortax/zgui (`crates/zgui-style/src/driver/recalc.rs`).

## 3. Preview Fixture Fonts (`SIL OFL 1.1`)

Unmodified Noto font binaries bundled in `@hewel/zgui-build` (`sdk/preview/fonts/`) for
deterministic headless preview rendering are redistributed under the **SIL Open Font License 1.1**:

- `NotoSans-Regular.ttf` — Copyright 2015–2021 Google LLC (`preview/fonts/NotoSans-LICENSE.txt`)
- `NotoSansCJKsc-Regular.otf` — Copyright 2014–2021 Adobe (`preview/fonts/NotoSansCJK-LICENSE.txt`)

See `preview/fonts/PROVENANCE.md` for exact upstream revisions and SHA-256 digests.
