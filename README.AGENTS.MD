# ttf-rasterizer — Agent Notes

This tool is a focused helper for terminal-font previewing, text raster experiments, and glyph asset export.

## Canonical doc

- `README.md`

## Main use cases

- preview terminal-friendly font rendering modes
- generate banner/text output files
- export glyph packs into mod asset folders

## Commands to know

- `cargo run -p ttf-rasterizer`
- `cargo run -- preview --font "JetBrains Mono" --mode terminal-pixels --size 28`
- `cargo run -- generate --font "JetBrains Mono" --text "SHELL QUEST" --output out/title.txt`
- `cargo run -- export-glyphs --font "JetBrains Mono" --size 24 --chars "AaBb0123" --output-dir ../../mods/base/assets/fonts`

## Working rules

- Keep output layouts and asset naming stable when possible.
- If export layout changes, update the README examples and any consuming asset docs together.

## Related local docs

- `../README.AGENTS.MD`
