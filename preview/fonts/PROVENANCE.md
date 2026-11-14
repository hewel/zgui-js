# Preview fixture fonts

These are unmodified upstream font binaries, redistributed under the SIL Open Font License 1.1. The adjacent license files reproduce the licenses at the pinned source revisions. Copyright and license statements are also embedded in each font's `name` table.

| Local file | Upstream revision and source | Embedded version and copyright | SHA-256 |
| --- | --- | --- | --- |
| `NotoSans-Regular.ttf` | [notofonts/noto-fonts, ffebf8c1ee449e544955a7e813c54f9b73848eac](https://github.com/notofonts/noto-fonts/blob/ffebf8c1ee449e544955a7e813c54f9b73848eac/hinted/ttf/NotoSans/NotoSans-Regular.ttf) | Version 2.008; Copyright 2015–2021 Google LLC. All Rights Reserved. | `b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5` |
| `NotoSansCJKsc-Regular.otf` | [notofonts/noto-cjk, f8d157532fbfaeda587e826d4cd5b21a49186f7c](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf) | Version 2.004; © 2014–2021 Adobe (http://www.adobe.com/). | `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b` |

Original license sources:

- [NotoSans-LICENSE.txt](https://github.com/notofonts/noto-fonts/blob/ffebf8c1ee449e544955a7e813c54f9b73848eac/LICENSE)
- [NotoSansCJK-LICENSE.txt](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/LICENSE)

`fonts.json` registers these under the application aliases `Preview Latin` and `Preview CJK`, in that fallback order. Aliases do not modify the original fonts. The Latin font has `A` and `B`; the CJK font has both `中` (U+4E2D) and `二` (U+4E8C), used by the existing editing example. No emoji font is needed by that example. These fixtures intentionally do not enumerate system fonts. The Latin binary is acquired directly from its licensed upstream source rather than assuming the different zgui test-fixture binary has the same provenance.

## Profile resource and evidence contract

Both `../profiles/default.json` (virtual time) and `../profiles/wall-static.json` (wall time) declare this same manifest. A manifest establishes an explicit independent resource root. Face and license paths are relative to its directory; traversal and canonical symlink escapes are rejected. Font content hashes are mandatory, and license text must be present, nonempty UTF-8. Registration failures never fall back to system fonts.

Profile and manifest JSON are each bounded to 64 KiB; individual fonts to 32 MiB, all fonts to 64 MiB, and each license to 64 KiB. Surface extents are rounded half up; physical dimensions must be 1–16384 per axis and at most 16777216 pixels total (64 MiB for RGBA8). Logical dimensions are 1–16384; scale is finite and in (0, 8]. Supported themes are `dark` and `light`; locale, time zone and direction are exactly `en-US`, `UTC`, and `ltr`. This declares the supported preview environment, not arbitrary system locale discovery or international formatter support.

All time inputs are integers. Epoch milliseconds are in 0–8640000000000000, start microseconds in 0–9007199254740991, and the configured virtual duration must not push either representation beyond its range. The schema documents individual watchdog, work, and output bounds. Physical-pixel and clock arithmetic constraints are checked by the loader before allocation.

The loader returns a reusable resolved profile object with CLI overrides applied, separately from verified font evidence. Its digest is the SHA-256 of compact UTF-8 JSON for `{ "profile": resolvedProfile, "fontEvidence": fontEvidence }`, with every object's keys recursively sorted, arrays retaining declared order, and no machine-specific absolute resource paths. Font evidence binds actual font hashes, license hashes, ordered fallback, physical/effective logical dimensions and timing determinism. The returned `font_family_css` is an escaped ordered CSS **value**; apply it through `:root { font-family: VALUE; }` after package styles before mounting. Wall mode is explicitly marked `timingDeterministic: false` and is not suitable for strict timing baselines. A virtual declaration alone is not proof that its consuming runner supports virtual time; the runner must reject unsupported capabilities.
