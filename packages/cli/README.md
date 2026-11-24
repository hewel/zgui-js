# `@hewel/zgui-cli`

Official Bun CLI (`zgui-js`) for creating, developing, building, previewing, inspecting, and packaging `zgui-js` native desktop applications and Wayland shells.

## Install

```sh
bun add -d @hewel/zgui-cli
```

On Linux x64 glibc (`ubuntu-24.04` / glibc >= 2.39 baseline), Bun installs `@hewel/zgui-host-linux-x64` automatically via `optionalDependencies`.

## Commands

- `zgui-js init [DIR] [--template desktop-app|wayland-shell]` — scaffold a new application.
- `zgui-js dev` — watch, type-check with `tsrs`, and live-reload inside `zgui-host`.
- `zgui-js build [--watch] [--bytecode]` — build an immutable SHA-256-sealed bundle.
- `zgui-js preview` — run headless GPU preview capture scenarios.
- `zgui-js package --out DIR [--bytecode] [--desktop-entry] [--pure-shell]` — assemble a self-contained runnable Linux directory including `licenses/`.
- `zgui-js doctor [--json]` — verify Bun, `zgui-host` ABI, Wayland, Vulkan, portal, `tsrs`, and `oxlint`.
- `zgui-js inspect [--ui] --socket PATH` — inspect a live `--dev-inspect` session or open the bundled Inspector UI.
- `zgui-js smoke` — run the compiler/runtime smoke build check.

## License

Apache-2.0. See `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md`.
