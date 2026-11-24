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
bun run dev
```

## Options

- `--template <desktop-app|wayland-shell>` — choose project template (default: `desktop-app`).
- `--name <APP_NAME>` — override the generated package and application identifier.
- `--force` — allow scaffolding into a non-empty target directory.

## License

Apache-2.0. See `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md`.
