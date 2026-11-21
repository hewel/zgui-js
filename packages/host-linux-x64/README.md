# `@hewel/zgui-host-linux-x64`

Prebuilt `native-host` runtime binary for `zgui-js` on Linux x64 (`x86_64`).
Installed automatically as an optional dependency of `@hewel/zgui-cli`.

To stage a locally built binary into this package before publishing:

```bash
bun tools/stage-host-package.ts --platform linux-x64 --binary target/release/native-host
```
