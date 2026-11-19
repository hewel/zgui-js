# Preview contracts & pixel inspection

Minimal valid v1 schemas for `profile.json`, `fixture.json`, and `scenario.json` (authoritative JSON Schemas live in `zgui-js/schemas/preview/*.schema.json`).

## Transparent pure-shell `profile.json`

Set `"pureShell": false` for standard `main`-window apps, or `"opaque": true` with `"background": "#10141dff"` for opaque surfaces:

```json
{
  "schemaVersion": 1,
  "id": "shell-transparent-800x600",
  "pureShell": true,
  "surface": {
    "name": "main",
    "width": 800,
    "height": 600,
    "scale": 1,
    "refreshRateMilliHz": 60000,
    "opaque": false,
    "background": "#00000000"
  },
  "theme": "dark",
  "reducedMotion": false,
  "locale": "en-US",
  "direction": "ltr",
  "timeZone": "UTC",
  "clock": { "mode": "virtual", "epochMs": 1700000000000, "startUs": 0 },
  "random": { "seed": 1 },
  "fonts": { "mode": "shipped-only", "manifest": "../fonts/fonts.json" },
  "assets": { "mode": "package-only", "readiness": "required-visible" },
  "limits": {
    "maxRunWallMs": 30000,
    "maxStepWallMs": 10000,
    "maxJsEntryWallMs": 1000,
    "maxJobsPerPump": 10000,
    "maxPumpWallMs": 1000,
    "maxSettleTurns": 1000,
    "maxFramesPerStep": 1000,
    "maxVirtualDurationMs": 60000,
    "maxOutputBytes": 67108864
  }
}
```

- `fonts.manifest` resolves relative to the profile JSON directory (point at `zgui-js/preview/fonts/fonts.json`).

## `fixture.json`

Empty minimal fixture: `{ "schemaVersion": 1, "id": "empty", "services": {}, "events": [] }`.

When mocking external services, declare only what the app calls (unconsumed socket/command scripts fail teardown):

```json
{
  "schemaVersion": 1,
  "id": "shell-services",
  "runtimeEnvironment": {
    "xdgRuntimeDir": "/run/user/1000",
    "homeDir": "/home/test",
    "xdgDataHome": "/home/test/.local/share",
    "waylandDisplay": "wayland-1",
    "namedSockets": { "NIRI_SOCKET": "/run/user/1000/niri.sock" }
  },
  "services": {
    "sockets": [
      {
        "path": "/run/user/1000/niri.sock",
        "framing": "line",
        "send": [],
        "events": [
          { "kind": "message", "atMs": 0, "text": "{\"workspaces\":[1,2]}" }
        ]
      }
    ],
    "commands": [
      {
        "program": "/usr/bin/wpctl",
        "args": ["get-volume", "@DEFAULT_AUDIO_SINK@"],
        "mode": "buffered",
        "events": [
          { "kind": "output", "atMs": 5, "status": 0, "stdout": "Volume: 0.65\n", "stderr": "" }
        ]
      }
    ],
    "control": {},
    "sessionLock": { "outcome": "locked" }
  },
  "events": []
}
```

- `services.images` maps `files` (`"file:///..." -> "<manifest-asset>"`) and `blobs` (`"<asset-path>" -> "<manifest-asset>"`).
- `services.sessionLock.outcome` is `"locked"` or `"finished"` (compositor refusal).

## `scenario.json`

All step `id`s must match `^[a-z0-9][a-z0-9._-]*$` and be unique:

```json
{
  "schemaVersion": 1,
  "id": "bar-and-popup",
  "requiredCapabilities": ["pixels", "capture.transparent", "surface.multiple", "virtual-time"],
  "steps": [
    {
      "id": "outputs",
      "op": "monitors.set",
      "monitors": [
        { "id": "dp-1", "name": "DP-1", "x": 0, "y": 0, "width": 1280, "height": 720, "scale": 1, "refreshRateMilliHz": 60000 }
      ]
    },
    { "id": "ready", "op": "settle" },
    { "id": "bar-open", "op": "capture", "surface": "bar-dp-1", "captureId": "bar-initial" },
    { "id": "click-menu", "op": "pointer.click", "target": { "surface": "bar-dp-1", "testId": "menu-btn" }, "button": "primary" },
    { "id": "tick", "op": "clock.advance", "byMs": 200 },
    { "id": "popup-shot", "op": "capture", "surface": "wifi-menu", "captureId": "popup-open" },
    { "id": "dismiss", "op": "popupDone", "surface": "wifi-menu" },
    { "id": "check-closed", "op": "assert", "target": { "surface": "bar-dp-1", "testId": "menu-btn" }, "check": { "property": "exists", "equals": true } }
  ]
}
```

Other supported ops: `pointer.move`, `wheel` (`delta: { x, y, unit: "pixels" | "lines" }`, optional `phase`), `key.press`, `text.commit`, `service.emit`, `surface.resize`, `surface.scale`, `surface.colorScheme`, `surface.recreate`, and `assert` with `serviceCall`.

## Inspecting PNG pixels in Bun tests

Inside `zgui-js`, reuse `tests/preview/png.mjs` (`decodePng(bytes)` -> `{ width, height, rgba }`, `pixel(png, x, y)` -> `[r, g, b, a]`) and `tests/preview/harness.mjs` (`runPreview`, `readJson`, `writeJson`). For transparent captures:
- Uncovered `#00000000` background pixels decode to `[0, 0, 0, 0]`.
- Translucent overlays/shadows decode to straight-alpha `[r, g, b, a]` with `0 < a < 255`.
- Opaque foregrounds and `<SessionLockSurface>` pixels decode with `a === 255`.
