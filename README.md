# PolyRoom

A Lightroom-style library and viewer for 3D-print models (STL, 3MF, OBJ), written in pure Rust
with [egui](https://github.com/emilk/egui) and [wgpu](https://wgpu.rs).

- Import folders; models are referenced in place, never copied.
- Thumbnail grid and filmstrip with rendered previews (cached on disk).
- 3D loupe: orbit, pan, zoom, with a build-plate grid.
- Lightroom keys: `0`–`5` rate, `P`/`X`/`U` flag, `6`–`9` colour labels, arrows to navigate,
  `G` grid, `E` loupe, `Tab` hide panels, `Ctrl+/` for all shortcuts.
- Metadata: dimensions in mm, triangle count, volume and surface area.

## Build & run

```bash
cargo run --release -p polyroom
```

```bash
cargo run --release -p polyroom -- --import "D:\3D Models"
```

The catalog lives in `%APPDATA%\PolyRoom` and thumbnails in `%LOCALAPPDATA%\PolyRoom`. Set
`POLYROOM_HOME` to keep both somewhere else, e.g. for testing.

## Development

```bash
cargo xtask ci
```

```bash
cargo xtask samples target/samples
```

`ci` runs fmt, clippy `-D warnings`, the tests and the layer check; `samples` writes a folder of
generated models (plus some deliberately broken files) to try the app with. See
[AGENTS.md](AGENTS.md) for the architecture rules.

## Layout

| Crate              | Purpose                                                        |
|--------------------|----------------------------------------------------------------|
| `crates/mesh`      | Indexed meshes, bounds, volume/area stats                      |
| `crates/formats`   | STL (binary/ASCII), OBJ and 3MF loaders                        |
| `crates/render`    | wgpu renderer for the loupe and headless thumbnails            |
| `crates/catalog`   | The library: models, ratings, flags, labels; op-log persistence |
| `crates/preview`   | Background loading, stats and thumbnail cache                  |
| `crates/engine`    | Session state and the command registry                         |
| `crates/ui-egui`   | The egui front-end                                             |
| `apps/polyroom`    | Desktop binary                                                 |
| `xtask`            | Build automation                                               |