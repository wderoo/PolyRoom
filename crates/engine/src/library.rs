//! Library commands: importing, choosing what the grid shows, selection, and rating/flagging.
//! Shortcuts follow Lightroom: 0–5 rate, P/X/U flag, 6–9 colour labels, arrows navigate.

use std::path::PathBuf;

use polyroom_catalog::{ColorLabel, Flag, ModelId, Op};
use serde_json::Value;

use crate::command::{Command, CommandRegistry, Key, Shortcut, always};
use crate::{Collection, EngineError, Session};

fn arg_str<'a>(command: &str, args: &'a Value, key: &str) -> Result<&'a str, EngineError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::InvalidArgument {
            command: command.into(),
            reason: format!("expected {{\"{key}\": string}}"),
        })
}

fn has_target(s: &Session) -> bool {
    s.current.is_some() || !s.selection.is_empty()
}

fn has_models(s: &Session) -> bool {
    !s.visible().is_empty()
}

fn rate(s: &mut Session, rating: u8) {
    for id in s.targets() {
        s.apply(Op::SetRating { id, rating });
    }
}

fn flag(s: &mut Session, flag: Flag) {
    for id in s.targets() {
        s.apply(Op::SetFlag { id, flag });
    }
}

/// Sets `label` on all targets, or clears it if the current model already has it (a toggle,
/// as in Lightroom).
fn label(s: &mut Session, label: ColorLabel) {
    let already = s
        .current
        .and_then(|id| s.model(id))
        .is_some_and(|m| m.label == Some(label));
    let label = (!already).then_some(label);
    for id in s.targets() {
        s.apply(Op::SetLabel { id, label });
    }
}

/// Moves the current model by `delta` positions in the visible list.
fn step(s: &mut Session, delta: isize) {
    let visible = s.visible();
    if visible.is_empty() {
        return;
    }
    let pos = s
        .current
        .and_then(|c| visible.iter().position(|&id| id == c));
    let next = match pos {
        Some(p) => p.saturating_add_signed(delta).min(visible.len() - 1),
        None => 0,
    };
    if let Some(&id) = visible.get(next) {
        select_only(s, id);
    }
}

fn select_only(s: &mut Session, id: ModelId) {
    s.current = Some(id);
    s.anchor = Some(id);
    s.selection.clear();
    s.selection.insert(id);
}

/// `{"id": n, "mode": "replace" | "toggle" | "extend"}` — a click, Ctrl+click or Shift+click.
fn select_model(s: &mut Session, args: &Value) -> Result<(), EngineError> {
    const ID: &str = "select.model";
    let id: ModelId =
        args.get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::InvalidArgument {
                command: ID.into(),
                reason: "expected {\"id\": number}".into(),
            })?;
    if !s.visible().contains(&id) {
        return Err(EngineError::InvalidArgument {
            command: ID.into(),
            reason: format!("model {id} is not shown"),
        });
    }
    match args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("replace")
    {
        "toggle" => {
            if !s.selection.remove(&id) {
                s.selection.insert(id);
                s.current = Some(id);
            } else if s.current == Some(id) {
                s.current = s.selection.iter().next().copied();
            }
            s.anchor = Some(id);
        }
        "extend" => {
            let visible = s.visible();
            let anchor = s.anchor.unwrap_or(id);
            let a = visible.iter().position(|&m| m == anchor);
            let b = visible.iter().position(|&m| m == id);
            if let (Some(a), Some(b)) = (a, b) {
                let range: Vec<ModelId> = visible
                    .get(a.min(b)..=a.max(b))
                    .unwrap_or_default()
                    .to_vec();
                s.selection = range.into_iter().collect();
            }
            s.current = Some(id);
        }
        _ => select_only(s, id),
    }
    Ok(())
}

macro_rules! simple {
    ($r:ident, $id:literal, $label:literal, $menu:literal, $shortcut:expr, $enabled:expr, |$s:ident| $body:expr) => {
        $r.register(Command {
            id: $id,
            label: $label,
            menu: $menu,
            shortcut: $shortcut,
            enabled: $enabled,
            run: |$s, _| {
                $body;
                Ok(())
            },
        });
    };
}

pub fn register(r: &mut CommandRegistry) {
    r.register(Command {
        id: "library.import",
        label: "Import Folder",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let path = arg_str("library.import", args, "path")?;
            s.start_import(PathBuf::from(path));
            Ok(())
        },
    });

    // What the grid shows.
    simple!(r, "library.show_all", "All Models", "", None, always, |s| {
        s.collection = Collection::All;
    });
    simple!(
        r,
        "library.show_previous_import",
        "Previous Import",
        "",
        None,
        always,
        |s| {
            s.collection = Collection::PreviousImport;
        }
    );
    r.register(Command {
        id: "library.show_folder",
        label: "Show Folder",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let path = arg_str("library.show_folder", args, "path")?;
            s.collection = Collection::Folder(PathBuf::from(path));
            Ok(())
        },
    });
    r.register(Command {
        id: "library.filter",
        label: "Filter",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            s.filter = arg_str("library.filter", args, "text")?.to_owned();
            Ok(())
        },
    });

    // Selection.
    r.register(Command {
        id: "select.model",
        label: "Select Model",
        menu: "",
        shortcut: None,
        enabled: always,
        run: select_model,
    });
    simple!(
        r,
        "select.next",
        "Next Model",
        "",
        Some(Shortcut::key(Key::Right)),
        has_models,
        |s| step(s, 1)
    );
    simple!(
        r,
        "select.previous",
        "Previous Model",
        "",
        Some(Shortcut::key(Key::Left)),
        has_models,
        |s| step(s, -1)
    );
    simple!(
        r,
        "select.down",
        "Model Below",
        "",
        Some(Shortcut::key(Key::Down)),
        has_models,
        |s| {
            let cols = s.grid_columns.max(1) as isize;
            step(
                s,
                if s.view == crate::ViewMode::Grid {
                    cols
                } else {
                    1
                },
            )
        }
    );
    simple!(
        r,
        "select.up",
        "Model Above",
        "",
        Some(Shortcut::key(Key::Up)),
        has_models,
        |s| {
            let cols = s.grid_columns.max(1) as isize;
            step(
                s,
                if s.view == crate::ViewMode::Grid {
                    -cols
                } else {
                    -1
                },
            )
        }
    );
    simple!(
        r,
        "select.all",
        "Select All",
        "Edit/Select All",
        Some(Shortcut::ctrl(Key::Char('a'))),
        has_models,
        |s| {
            s.selection = s.visible().iter().copied().collect();
            if s.current.is_none() {
                s.current = s.visible().first().copied();
            }
        }
    );
    simple!(
        r,
        "select.none",
        "Select None",
        "Edit/Select None",
        Some(Shortcut::ctrl(Key::Char('d'))),
        has_target,
        |s| {
            s.selection.clear();
            s.selection.extend(s.current);
        }
    );

    // Ratings.
    simple!(
        r,
        "model.rate.0",
        "No Rating",
        "Model/Rating/None",
        Some(Shortcut::key(Key::Char('0'))),
        has_target,
        |s| rate(s, 0)
    );
    simple!(
        r,
        "model.rate.1",
        "1 Star",
        "Model/Rating/1 Star",
        Some(Shortcut::key(Key::Char('1'))),
        has_target,
        |s| rate(s, 1)
    );
    simple!(
        r,
        "model.rate.2",
        "2 Stars",
        "Model/Rating/2 Stars",
        Some(Shortcut::key(Key::Char('2'))),
        has_target,
        |s| rate(s, 2)
    );
    simple!(
        r,
        "model.rate.3",
        "3 Stars",
        "Model/Rating/3 Stars",
        Some(Shortcut::key(Key::Char('3'))),
        has_target,
        |s| rate(s, 3)
    );
    simple!(
        r,
        "model.rate.4",
        "4 Stars",
        "Model/Rating/4 Stars",
        Some(Shortcut::key(Key::Char('4'))),
        has_target,
        |s| rate(s, 4)
    );
    simple!(
        r,
        "model.rate.5",
        "5 Stars",
        "Model/Rating/5 Stars",
        Some(Shortcut::key(Key::Char('5'))),
        has_target,
        |s| rate(s, 5)
    );

    // Flags.
    simple!(
        r,
        "model.flag.pick",
        "Flag as Pick",
        "Model/Flag/Pick",
        Some(Shortcut::key(Key::Char('p'))),
        has_target,
        |s| flag(s, Flag::Pick)
    );
    simple!(
        r,
        "model.flag.reject",
        "Flag as Rejected",
        "Model/Flag/Rejected",
        Some(Shortcut::key(Key::Char('x'))),
        has_target,
        |s| flag(s, Flag::Reject)
    );
    simple!(
        r,
        "model.flag.none",
        "Unflag",
        "Model/Flag/Unflagged",
        Some(Shortcut::key(Key::Char('u'))),
        has_target,
        |s| flag(s, Flag::None)
    );

    // Colour labels.
    simple!(
        r,
        "model.label.red",
        "Red Label",
        "Model/Color Label/Red",
        Some(Shortcut::key(Key::Char('6'))),
        has_target,
        |s| label(s, ColorLabel::Red)
    );
    simple!(
        r,
        "model.label.yellow",
        "Yellow Label",
        "Model/Color Label/Yellow",
        Some(Shortcut::key(Key::Char('7'))),
        has_target,
        |s| label(s, ColorLabel::Yellow)
    );
    simple!(
        r,
        "model.label.green",
        "Green Label",
        "Model/Color Label/Green",
        Some(Shortcut::key(Key::Char('8'))),
        has_target,
        |s| label(s, ColorLabel::Green)
    );
    simple!(
        r,
        "model.label.blue",
        "Blue Label",
        "Model/Color Label/Blue",
        Some(Shortcut::key(Key::Char('9'))),
        has_target,
        |s| label(s, ColorLabel::Blue)
    );
    simple!(
        r,
        "model.label.purple",
        "Purple Label",
        "Model/Color Label/Purple",
        None,
        has_target,
        |s| label(s, ColorLabel::Purple)
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use polyroom_catalog::ScannedFile;
    use polyroom_formats::Format;
    use serde_json::json;

    use crate::{Collection, Session, default_registry};

    fn session_with(names: &[&str]) -> Session {
        let mut s = Session::new();
        let files = names
            .iter()
            .map(|n| ScannedFile {
                path: Path::new("C:/m").join(n),
                format: Format::Stl,
                size: 1,
                modified: None,
            })
            .collect();
        s.catalog.import(Path::new("C:/m"), files).unwrap();
        s.poll();
        s
    }

    #[test]
    fn rating_flag_and_label_apply_to_selection() {
        let r = default_registry();
        let mut s = session_with(&["a.stl", "b.stl", "c.stl"]);
        r.run("select.model", &mut s, &json!({"id": 0})).unwrap();
        r.run("select.model", &mut s, &json!({"id": 2, "mode": "extend"}))
            .unwrap();
        assert_eq!(s.selection_len(), 3);
        r.run("model.rate.4", &mut s, &json!(null)).unwrap();
        r.run("model.flag.pick", &mut s, &json!(null)).unwrap();
        r.run("model.label.green", &mut s, &json!(null)).unwrap();
        for m in s.catalog().models() {
            assert_eq!(m.rating, 4);
            assert_eq!(m.flag, polyroom_catalog::Flag::Pick);
            assert_eq!(m.label, Some(polyroom_catalog::ColorLabel::Green));
        }
        // Applying the same label again clears it.
        r.run("model.label.green", &mut s, &json!(null)).unwrap();
        assert!(s.catalog().models().iter().all(|m| m.label.is_none()));
    }

    #[test]
    fn rating_needs_a_target() {
        let r = default_registry();
        let mut s = session_with(&["a.stl"]);
        s.current = None;
        s.selection.clear();
        assert!(r.run("model.rate.3", &mut s, &json!(null)).is_err());
    }

    #[test]
    fn arrow_navigation_respects_grid_columns() {
        let r = default_registry();
        let mut s = session_with(&["a.stl", "b.stl", "c.stl", "d.stl", "e.stl"]);
        s.grid_columns = 2;
        r.run("select.model", &mut s, &json!({"id": 0})).unwrap();
        r.run("select.down", &mut s, &json!(null)).unwrap();
        assert_eq!(s.current(), Some(2));
        r.run("select.next", &mut s, &json!(null)).unwrap();
        assert_eq!(s.current(), Some(3));
        r.run("select.down", &mut s, &json!(null)).unwrap();
        assert_eq!(s.current(), Some(4)); // clamped at the end
        r.run("select.up", &mut s, &json!(null)).unwrap();
        assert_eq!(s.current(), Some(2));
        assert_eq!(s.selection_len(), 1);
    }

    #[test]
    fn ctrl_click_toggles() {
        let r = default_registry();
        let mut s = session_with(&["a.stl", "b.stl"]);
        r.run("select.model", &mut s, &json!({"id": 0})).unwrap();
        r.run("select.model", &mut s, &json!({"id": 1, "mode": "toggle"}))
            .unwrap();
        assert_eq!(s.selection_len(), 2);
        r.run("select.model", &mut s, &json!({"id": 0, "mode": "toggle"}))
            .unwrap();
        assert_eq!(s.selection_len(), 1);
        assert!(s.is_selected(1));
    }

    #[test]
    fn filter_and_collections_narrow_the_grid() {
        let r = default_registry();
        let mut s = session_with(&["Dragon Head.stl", "dragon tail.stl", "boat.stl"]);
        r.run("library.filter", &mut s, &json!({"text": "DRAGON"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 2);
        r.run("library.filter", &mut s, &json!({"text": "dragon head"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 1);
        r.run("library.filter", &mut s, &json!({"text": ""}))
            .unwrap();
        r.run(
            "library.show_folder",
            &mut s,
            &json!({"path": "C:/elsewhere"}),
        )
        .unwrap();
        s.poll();
        assert!(s.visible().is_empty());
        assert_eq!(s.collection(), &Collection::Folder("C:/elsewhere".into()));
    }

    #[test]
    fn selecting_a_hidden_model_is_rejected() {
        let r = default_registry();
        let mut s = session_with(&["a.stl"]);
        assert!(r.run("select.model", &mut s, &json!({"id": 99})).is_err());
    }
}
