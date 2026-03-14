# ttf-rasterizer

Small Rust CLI for terminal font sprite previews and generation.

## Use
- `cargo run -- list`
- `cargo run -- preview --font "JetBrains Mono"` (defaults to alphanumerics)
- `cargo run -- preview --font "JetBrains Mono" --chars "AaBbCc123"`
- `cargo run -- preview --font "JetBrains Mono" --mode terminal-pixels --size 28`
- `cargo run -- preview --font "JetBrains Mono" --mode cell --size 28` (alias of `terminal-pixels`)
- `cargo run -- generate --font "JetBrains Mono" --text "SHELL QUEST" --output out/title.txt`
- `cargo run -- generate --font "JetBrains Mono" --mode terminal-pixels --chars "█▓▒░" --output out/tiles.txt`
- `cargo run -- export-glyphs --font "JetBrains Mono" --size 24 --chars "AaBb0123" --output-dir ../../mods/base/assets/fonts`

## Glyph asset export

`export-glyphs` writes a real asset pack:

- `mods/base/assets/fonts/{size}px/{font-name}/manifest.yaml`
- `mods/base/assets/fonts/{size}px/{font-name}/glyphs/<char>.txt`

Examples:

- `A` -> `glyphs/A.txt`
- `a` -> `glyphs/a.txt`
- `2` -> `glyphs/2.txt`
- non-alnum -> safe names like `glyphs/U+003F.txt`
