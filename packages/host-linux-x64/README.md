# `@hewel/zgui-host-linux-x64`

Prebuilt `zgui-host` native runtime binary for `zgui-js` on Linux x64 (`x86_64`, glibc).
Installed automatically as an optional dependency of `@hewel/zgui-cli`.

## Runtime Baseline

- **Target:** Linux `x86_64` with GNU `glibc` (`"libc": ["glibc"]`; musl distributions are excluded).
- **Build baseline:** `ubuntu-24.04` (`glibc >= 2.39`).
- **Graphics / desktop libraries:** requires system Wayland / Vulkan / Fontconfig / FreeType / `libxkbcommon` runtime libraries (or Mesa `llvmpipe` for headless previews).

## Staging Locally

To stage a locally compiled binary into this package before packing:

```sh
bun tools/stage-host-package.ts --platform linux-x64 --binary target/release/zgui-host
```

## License & Notices

Apache-2.0 with file-scoped MPL-2.0 (`vendor/zgui-style/src/driver/recalc.rs`). See `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md`.
