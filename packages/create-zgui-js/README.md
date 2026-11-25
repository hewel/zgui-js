# `create-zgui-js`

Project scaffolder for `zgui-js` native desktop applications and Wayland shells.

## Usage

```sh
bun create zgui-js my-app
bun create zgui-js my-shell --template wayland-shell
```

Then install dependencies and start developing:

```sh
cd my-app
bun install
bunx zgui-js doctor
bun run dev
```

On Linux `x86_64` (`glibc >= 2.39`), `bun install` automatically downloads the prebuilt `@hewel/zgui-host-linux-x64` runtime via `@hewel/zgui-cli`'s `optionalDependencies` — no Rust toolchain is required.

## Options

- `--template <desktop-app|wayland-shell>` — choose project template (default: `desktop-app`).
- `--name <APP_NAME>` — override the generated package and application identifier.
- `--force` — allow scaffolding into a non-empty target directory.

## License

Apache-2.0. See `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md`.
