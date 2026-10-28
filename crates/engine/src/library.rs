//! Library commands: importing, choosing what the grid shows, selection, and rating/flagging.
//! Shortcuts follow Lightroom: 0–5 rate, P/X/U flag, 6–9 colour labels, arrows navigate.

use std::path::PathBuf;

use polyroom_catalog::{ColorLabel, Flag, ModelId, Op};
use polyroom_formats::Format;
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

fn unknown(command: &str, key: &str, value: &str) -> EngineError {
    EngineError::InvalidArgument {
        command: command.into(),
        reason: format!("unknown {key} \"{value}\""),
    }
}

/// The folder being browsed, if any.
fn current_folder(s: &Session) -> Option<PathBuf> {
    match s.collection() {
        Collection::Folder(p) => Some(p.clone()),
        _ => None,
    }
}

/// `{"path": "..."}`, or the folder being browsed when no path is given (menu, shortcut).
fn folder_arg(command: &str, s: &Session, args: &Value) -> Result<PathBuf, EngineError> {
    match args.get("path") {
        Some(_) => arg_str(command, args, "path").map(PathBuf::from),
        None => current_folder(s).ok_or_else(|| EngineError::InvalidArgument {
            command: command.into(),
            reason: "no folder given and none is open".into(),
        }),
    }
}

/// The folder above the one being browsed, if that is still inside its imported folder.
fn parent_folder(s: &Session) -> Option<PathBuf> {
    let Collection::Folder(current) = s.collection() else {
        return None;
    };
    let root = s.folder_root(current)?;
    (current.as_path() != root)
        .then(|| current.parent().map(PathBuf::from))
        .flatten()
}

fn has_target(s: &Session) -> bool {
    s.current.is_some() || !s.selection.is_empty()
}

fn has_models(s: &Session) -> bool {
    !s.visible().is_empty()
}

fn has_stack(s: &Session) -> bool {
    s.current.is_some_and(|id| !s.stack(id).is_empty())
}

/// Shows `photo` (a photo from the current model's stack) in the loupe, or the 3D view for
/// `None`.
fn show_photo(s: &mut Session, photo: Option<ModelId>) {
    // Make sure the loupe holds the current model, so the choice isn't reset right after.
    s.update_loupe();
    s.loupe.photo = photo;
    s.update_loupe();
}

/// Steps through the stack's photos, then back to the 3D view.
fn next_photo(s: &mut Session) {
    let Some(current) = s.current else {
        return;
    };
    let stack = s.stack(current);
    let next = match s
        .loupe
        .photo
        .and_then(|p| stack.iter().position(|&i| i == p))
    {
        Some(i) => stack.get(i + 1).copied(),
        None => stack.first().copied(),
    };
    show_photo(s, next);
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
    simple!(
        r,
        "library.show_all",
        "All Models",
        "Library/All Models",
        None,
        always,
        |s| {
            s.collection = Collection::All;
        }
    );
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
        id: "library.refresh_folder",
        label: "Refresh Folder",
        menu: "Library/Refresh Folder",
        shortcut: Some(Shortcut::key(Key::F5)),
        enabled: |s| current_folder(s).is_some(),
        run: |s, args| {
            let folder = folder_arg("library.refresh_folder", s, args)?;
            s.start_refresh(folder)
                .map_err(|reason| EngineError::InvalidArgument {
                    command: "library.refresh_folder".into(),
                    reason,
                })
        },
    });
    r.register(Command {
        id: "library.show_in_explorer",
        label: "Show in Explorer",
        menu: "Library/Show in Explorer",
        shortcut: None,
        enabled: |s| current_folder(s).is_some(),
        run: |s, args| {
            let folder = folder_arg("library.show_in_explorer", s, args)?;
            s.request(crate::Request::ShowInExplorer(folder));
            Ok(())
        },
    });
    r.register(Command {
        id: "library.folder_up",
        label: "Up One Folder",
        menu: "Library/Up One Folder",
        shortcut: Some(Shortcut::key(Key::Backspace)),
        enabled: |s| parent_folder(s).is_some(),
        run: |s, _| {
            if let Some(parent) = parent_folder(s) {
                s.collection = Collection::Folder(parent);
                s.view = crate::ViewMode::Grid;
            }
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

    // Metadata filter. Each command toggles one choice; `filter.clear` resets them all.
    r.register(Command {
        id: "filter.rating",
        label: "Filter by Rating",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let stars = args
                .get("min")
                .and_then(Value::as_u64)
                .filter(|n| *n <= 5)
                .ok_or_else(|| EngineError::InvalidArgument {
                    command: "filter.rating".into(),
                    reason: "expected {\"min\": 0-5}".into(),
                })?;
            s.attributes.toggle_min_rating(stars as u8);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.flag",
        label: "Filter by Flag",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let flag = match arg_str("filter.flag", args, "flag")? {
                "pick" => Flag::Pick,
                "reject" => Flag::Reject,
                "none" => Flag::None,
                other => return Err(unknown("filter.flag", "flag", other)),
            };
            s.attributes.toggle_flag(flag);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.label",
        label: "Filter by Color Label",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let name = arg_str("filter.label", args, "label")?;
            let label = ColorLabel::ALL
                .into_iter()
                .find(|l| l.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| unknown("filter.label", "label", name))?;
            s.attributes.toggle_label(label);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.format",
        label: "Filter by File Format",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let name = arg_str("filter.format", args, "format")?;
            let format = Format::ALL
                .into_iter()
                .find(|f| f.label().eq_ignore_ascii_case(name))
                .ok_or_else(|| unknown("filter.format", "format", name))?;
            s.attributes.toggle_format(format);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.clear",
        label: "Clear Attribute Filters",
        menu: "Library/Clear Attribute Filters",
        shortcut: None,
        enabled: |s| s.attributes.is_active(),
        run: |s, _| {
            s.attributes.clear();
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

    // Photos stacked on a model, shown in the loupe.
    r.register(Command {
        id: "loupe.show_photo",
        label: "Show Photo",
        menu: "",
        shortcut: None,
        enabled: has_stack,
        run: |s, args| {
            const ID: &str = "loupe.show_photo";
            let id = args.get("id").and_then(Value::as_u64).ok_or_else(|| {
                EngineError::InvalidArgument {
                    command: ID.into(),
                    reason: "expected {\"id\": number}".into(),
                }
            })?;
            let in_stack = s.current.is_some_and(|c| s.stack(c).contains(&id));
            if !in_stack {
                return Err(EngineError::InvalidArgument {
                    command: ID.into(),
                    reason: format!("{id} is not a photo of the current model"),
                });
            }
            show_photo(s, Some(id));
            Ok(())
        },
    });
    simple!(
        r,
        "loupe.next_photo",
        "Next Photo",
        "View/Loupe/Next Photo",
        None,
        has_stack,
        |s| next_photo(s)
    );
    simple!(
        r,
        "loupe.show_3d",
        "Show 3D Model",
        "View/Loupe/Show 3D Model",
        None,
        |s| s.loupe.photo.is_some(),
        |s| show_photo(s, None)
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

    /// Like `session_with`, but each file gets the format its extension says.
    fn session_with_files(names: &[&str]) -> Session {
        let mut s = Session::new();
        let files = names
            .iter()
            .map(|n| {
                let path = Path::new("C:/m").join(n);
                ScannedFile {
                    format: Format::from_path(&path).unwrap(),
                    path,
                    size: 1,
                    modified: None,
                }
            })
            .collect();
        s.catalog.import(Path::new("C:/m"), files).unwrap();
        s.collection = Collection::Folder("C:/m".into());
        s.poll();
        s
    }

    fn visible_names(s: &Session) -> Vec<String> {
        s.visible()
            .iter()
            .filter_map(|&id| Some(s.model(id)?.name()))
            .collect()
    }

    fn id_of(s: &Session, name: &str) -> u64 {
        s.catalog()
            .models()
            .iter()
            .find(|m| m.name() == name)
            .unwrap()
            .id
    }

    #[test]
    fn stacked_photos_show_through_their_model() {
        let r = default_registry();
        let mut s =
            session_with_files(&["wolf.stl", "wolf.png", "boat.stl", "other.jpg", "kit.zip"]);
        assert_eq!(
            visible_names(&s),
            ["wolf.stl", "boat.stl", "other.jpg", "kit.zip"]
        );
        let (wolf, photo) = (id_of(&s, "wolf.stl"), id_of(&s, "wolf.png"));
        assert_eq!(s.stack(wolf), [photo]);
        assert_eq!(s.cover(wolf), photo);
        assert_eq!(s.cover(id_of(&s, "boat.stl")), id_of(&s, "boat.stl"));

        // Filtering by images hides the model, so its photo shows on its own.
        r.run("filter.format", &mut s, &json!({"format": "image"}))
            .unwrap();
        s.poll();
        assert_eq!(visible_names(&s), ["wolf.png", "other.jpg"]);
        r.run("filter.clear", &mut s, &json!(null)).unwrap();

        r.run("view.toggle_photo_covers", &mut s, &json!(null))
            .unwrap();
        assert_eq!(s.cover(wolf), wolf, "renders again");
    }

    #[test]
    fn the_loupe_steps_through_a_models_photos() {
        let r = default_registry();
        let mut s = session_with_files(&["wolf.stl", "wolf.png", "wolf 2.jpg", "boat.stl"]);
        let (wolf, p1, p2) = (
            id_of(&s, "wolf.stl"),
            id_of(&s, "wolf.png"),
            id_of(&s, "wolf 2.jpg"),
        );
        let boat = id_of(&s, "boat.stl");
        r.run("select.model", &mut s, &json!({"id": boat})).unwrap();
        assert!(!r.is_enabled("loupe.next_photo", &s), "boat has no photos");

        r.run("select.model", &mut s, &json!({"id": wolf})).unwrap();
        r.run("view.loupe", &mut s, &json!(null)).unwrap();
        s.poll();
        assert!(!r.is_enabled("loupe.show_3d", &s));
        let mut shown = Vec::new();
        for _ in 0..3 {
            r.run("loupe.next_photo", &mut s, &json!(null)).unwrap();
            s.poll();
            shown.push(s.loupe().photo);
        }
        assert_eq!(
            shown,
            [Some(p1), Some(p2), None],
            "photos, then the 3D view"
        );
        assert_eq!(s.loupe().id, Some(wolf));

        r.run("loupe.show_photo", &mut s, &json!({"id": p2}))
            .unwrap();
        assert_eq!(s.loupe().picture_of, Some(p2));
        r.run("loupe.show_3d", &mut s, &json!(null)).unwrap();
        assert_eq!(s.loupe().photo, None);
        assert!(
            r.run("loupe.show_photo", &mut s, &json!({"id": wolf}))
                .is_err(),
            "a model is not its own photo"
        );
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
    fn folders_show_own_models_and_subfolders() {
        let r = default_registry();
        let mut s = session_with(&[
            "top.stl",
            "Halloween/ghost.stl",
            "Halloween/skeleton/arm.stl",
        ]);
        r.run("library.show_folder", &mut s, &json!({"path": "C:/m"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 1);
        assert_eq!(s.visible_folders(), [Path::new("C:/m/Halloween")]);
        assert!(
            !r.is_enabled("library.folder_up", &s),
            "can't go above the import"
        );

        r.run(
            "library.show_folder",
            &mut s,
            &json!({"path": "C:/m/Halloween/skeleton"}),
        )
        .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 1);
        assert!(s.visible_folders().is_empty());
        r.run("library.folder_up", &mut s, &json!(null)).unwrap();
        s.poll();
        assert_eq!(s.collection(), &Collection::Folder("C:/m/Halloween".into()));

        // Searching a folder includes its subfolders and hides the folder cells.
        r.run("library.show_folder", &mut s, &json!({"path": "C:/m"}))
            .unwrap();
        r.run("library.filter", &mut s, &json!({"text": "arm"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 1);
        assert!(s.visible_folders().is_empty());
    }

    #[test]
    fn refresh_syncs_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("sub")).unwrap();
        for f in ["a.stl", "sub/b.stl", "sub/c.stl"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let r = default_registry();
        let mut s = Session::new();
        r.run("library.import", &mut s, &json!({"path": root}))
            .unwrap();
        wait_for_scans(&mut s);
        assert_eq!(s.catalog().models().len(), 3);

        // Change the disk: delete c, add d; then refresh the open "sub" folder via F5's command.
        std::fs::remove_file(root.join("sub/c.stl")).unwrap();
        std::fs::write(root.join("sub/d.stl"), b"x").unwrap();
        r.run(
            "library.show_folder",
            &mut s,
            &json!({"path": root.join("sub")}),
        )
        .unwrap();
        r.run("library.refresh_folder", &mut s, &json!(null))
            .unwrap();
        wait_for_scans(&mut s);
        let mut names: Vec<_> = s.catalog().models().iter().map(|m| m.name()).collect();
        names.sort();
        assert_eq!(names, ["a.stl", "b.stl", "d.stl"]);
        assert!(s.status().contains("1 added, 1 removed"), "{}", s.status());

        // A folder outside every import is refused.
        assert!(
            r.run(
                "library.refresh_folder",
                &mut s,
                &json!({"path": "Z:/nowhere"})
            )
            .is_err()
        );
    }

    #[test]
    fn refreshing_a_deleted_folder_falls_back_to_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("gone")).unwrap();
        std::fs::write(root.join("keep.stl"), b"x").unwrap();
        std::fs::write(root.join("gone/x.stl"), b"x").unwrap();
        let r = default_registry();
        let mut s = Session::new();
        r.run("library.import", &mut s, &json!({"path": root}))
            .unwrap();
        wait_for_scans(&mut s);
        r.run(
            "library.show_folder",
            &mut s,
            &json!({"path": root.join("gone")}),
        )
        .unwrap();
        std::fs::remove_dir_all(root.join("gone")).unwrap();
        r.run("library.refresh_folder", &mut s, &json!(null))
            .unwrap();
        wait_for_scans(&mut s);
        assert_eq!(s.collection(), &Collection::Folder(root.to_owned()));
        assert_eq!(s.catalog().models().len(), 1);
    }

    #[test]
    fn attribute_filters_narrow_the_grid_and_follow_edits() {
        let r = default_registry();
        let mut s = session_with(&["a.stl", "b.stl", "c.stl", "d.stl"]);
        let run = |s: &mut Session, id: &str, args: serde_json::Value| {
            r.run(id, s, &args).unwrap();
            s.poll();
        };
        // a: 5 stars, b: 3 stars + pick, c: 3 stars + red, d: untouched.
        for (id, rating) in [(0, 5), (1, 3), (2, 3)] {
            run(&mut s, "select.model", json!({"id": id}));
            run(&mut s, &format!("model.rate.{rating}"), json!(null));
        }
        run(&mut s, "select.model", json!({"id": 1}));
        run(&mut s, "model.flag.pick", json!(null));
        run(&mut s, "select.model", json!({"id": 2}));
        run(&mut s, "model.label.red", json!(null));
        let shown = |s: &Session| s.visible().to_vec();
        assert_eq!(shown(&s), [0, 1, 2, 3]);

        run(&mut s, "filter.rating", json!({"min": 3}));
        assert_eq!(shown(&s), [0, 1, 2], "3 stars or more");
        run(&mut s, "filter.flag", json!({"flag": "pick"}));
        assert_eq!(shown(&s), [1]);
        run(&mut s, "filter.flag", json!({"flag": "pick"})); // toggles off again
        run(&mut s, "filter.label", json!({"label": "Red"}));
        assert_eq!(shown(&s), [2]);
        run(&mut s, "filter.format", json!({"format": "obj"}));
        assert!(shown(&s).is_empty());
        run(&mut s, "filter.format", json!({"format": "stl"}));
        run(&mut s, "filter.format", json!({"format": "obj"}));
        assert_eq!(shown(&s), [2]);

        // Editing a model while filtered updates the grid right away.
        run(&mut s, "select.model", json!({"id": 2}));
        run(&mut s, "model.rate.1", json!(null));
        assert!(shown(&s).is_empty(), "no longer 3 stars");
        // (The edited model left the selection with it, so edit through the engine directly.)
        s.apply(polyroom_catalog::Op::SetRating { id: 2, rating: 4 });
        s.poll();
        assert_eq!(shown(&s), [2]);

        assert!(r.is_enabled("filter.clear", &s));
        run(&mut s, "filter.clear", json!(null));
        assert!(!r.is_enabled("filter.clear", &s));
        assert_eq!(shown(&s), [0, 1, 2, 3]);

        for (id, args) in [
            ("filter.rating", json!({"min": 6})),
            ("filter.flag", json!({"flag": "maybe"})),
            ("filter.label", json!({})),
            ("filter.format", json!({"format": "step"})),
        ] {
            assert!(r.run(id, &mut s, &args).is_err(), "{id} {args}");
        }
    }

    #[test]
    fn an_attribute_filter_searches_subfolders() {
        let r = default_registry();
        let mut s = session_with(&["top.stl", "sub/deep.stl"]);
        r.run("library.show_folder", &mut s, &json!({"path": "C:/m"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 1);
        assert_eq!(s.visible_folders().len(), 1);
        r.run("filter.format", &mut s, &json!({"format": "stl"}))
            .unwrap();
        s.poll();
        assert_eq!(s.visible().len(), 2);
        assert!(s.visible_folders().is_empty());
    }

    #[test]
    fn importing_clears_the_attribute_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.stl"), b"x").unwrap();
        let r = default_registry();
        let mut s = Session::new();
        r.run("filter.rating", &mut s, &json!({"min": 4})).unwrap();
        r.run("library.import", &mut s, &json!({"path": dir.path()}))
            .unwrap();
        wait_for_scans(&mut s);
        assert!(!s.attributes().is_active());
        assert_eq!(s.visible().len(), 1, "new, unrated models must be shown");
    }

    #[test]
    fn scanning_reports_indeterminate_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.stl"), b"x").unwrap();
        let r = default_registry();
        let mut s = Session::new();
        assert!(s.progress().is_none());
        r.run("library.import", &mut s, &json!({"path": dir.path()}))
            .unwrap();
        let p = s.progress().unwrap();
        assert_eq!(p.fraction, None);
        wait_for_scans(&mut s);
        assert!(s.progress().is_none());
    }

    fn wait_for_scans(s: &mut Session) {
        let start = std::time::Instant::now();
        while s.is_busy() && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.poll();
        }
        s.poll();
    }

    #[test]
    fn selecting_a_hidden_model_is_rejected() {
        let r = default_registry();
        let mut s = session_with(&["a.stl"]);
        assert!(r.run("select.model", &mut s, &json!({"id": 99})).is_err());
    }
}
