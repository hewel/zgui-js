# `@hewel/zgui-host-linux-arm64`

Prebuilt `native-host` runtime binary for `zgui-js` on Linux arm64 (`aarch64`).
Installed automatically as an optional dependency of `@hewel/zgui-cli`.

To stage a locally built binary into this package before publishing:

```bash
bun tools/stage-host-package.ts --platform linux-arm64 --binary target/release/native-host
```
