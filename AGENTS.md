# PolyRoom: rules for contributors and agents

PolyRoom is a Lightroom-style library and viewer for 3D-print models (STL, 3MF, OBJ, and later STEP).
It is pure Rust: egui for the UI, wgpu for rendering.

## Layering (enforced by `cargo xtask layers`)

| Layer | Crates                         | May depend on |
|-------|--------------------------------|---------------|
| L0    | `mesh`                         | external only |
| L1    | `formats`                      | L0            |
| L2    | `render`                       | L0–L1         |
| L3    | `catalog`, `preview`           | L0–L2         |
| L4    | `engine`                       | L0–L3         |
| L5    | `ui-egui`, apps                | anything      |

Nothing below L5 may depend on `egui`, `eframe`, `winit` or `rfd`. The engine owns all state and
behaviour. The UI only draws that state and dispatches commands.

## Everything is a command

Each user action is registered in `polyroom_engine::CommandRegistry` with a stable id
(`view.grid`, `model.rate.3`, ...), a label, a menu path, an optional shortcut, an `enabled` check
and a `run` function. Menus, keyboard shortcuts, the CLI and (later) MCP all dispatch by id, so they
behave identically.

Actions the engine cannot perform itself (opening a native dialog, quitting the window) are queued
as `Request`s for the front-end to fulfil.

## Never crash

Model files are hostile input. A malformed STL/3MF/OBJ, a corrupt catalog, or a full disk must
produce an error the user can act on, never a panic.

- No `unwrap`, `expect`, `panic!`, `todo!`, `unreachable!` or slice indexing in production code
  (clippy denies them workspace-wide; tests are exempt via `clippy.toml`). Use `.get()` and `?`.
- Use checked or saturating arithmetic on counts and offsets read from files.
- `unsafe` is forbidden everywhere.
- A provably infallible case may use `#[allow(clippy::expect_used)]` with a comment saying why.

## Assets

Only original or permissively licensed assets (CC0, CC-BY, OFL, MIT/Apache/BSD). No Adobe
assets, not even as placeholders. Watching Lightroom's layout and behaviour is fine; copying its
icons or imagery is not. Icons are drawn in code.

## Workflow

- `cargo xtask ci` (fmt check, clippy `-D warnings`, tests, layers) must pass before every commit.
- Each new feature comes with at least one test that would fail without it.
- Run the app after every user-visible change.
