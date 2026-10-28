//! Commands that organise the library: categories, tags, favorites, sorting, file actions and
//! removing folders. Edits apply to `{"path"}` when one is given (a folder from a context menu),
//! else to the folder clicked in the grid, else to the selected models.

use std::path::{Path, PathBuf};

use polyroom_catalog::{Category, CategoryId, Op, Target, clean_tag};
use serde_json::Value;

use crate::command::{Command, CommandRegistry, Key, Shortcut, always};
use crate::library::arg_str;
use crate::settings::THUMB_SIZES;
use crate::sort::SortKey;
use crate::{EngineError, Request, Session};

/// Colours handed to new categories in turn.
const PALETTE: [[u8; 3]; 6] = [
    [0x42, 0xa5, 0xf5],
    [0xab, 0x47, 0xbc],
    [0x26, 0xa6, 0x9a],
    [0xff, 0xca, 0x28],
    [0xec, 0x40, 0x7a],
    [0x78, 0x90, 0x9c],
];

fn invalid(command: &str, reason: impl Into<String>) -> EngineError {
    EngineError::InvalidArgument {
        command: command.into(),
        reason: reason.into(),
    }
}

/// The folder or models an edit applies to.
fn targets(command: &str, s: &Session, args: &Value) -> Result<Vec<Target>, EngineError> {
    let targets: Vec<Target> = if args.get("path").is_some() {
        vec![Target::Folder(PathBuf::from(arg_str(
            command, args, "path",
        )?))]
    } else if let Some(folder) = s.focused_folder() {
        vec![Target::Folder(folder.to_owned())]
    } else {
        s.targets().into_iter().map(Target::Model).collect()
    };
    if targets.is_empty() {
        return Err(invalid(command, "select a file or folder first"));
    }
    Ok(targets)
}

/// The files or folders an action applies to, as paths.
fn paths(command: &str, s: &Session, args: &Value) -> Result<Vec<PathBuf>, EngineError> {
    let paths: Vec<PathBuf> = targets(command, s, args)?
        .into_iter()
        .filter_map(|t| match t {
            Target::Folder(p) => Some(p),
            Target::Model(id) => s.model(id).map(|m| m.path.clone()),
        })
        .collect();
    if paths.is_empty() {
        return Err(invalid(command, "select a file or folder first"));
    }
    Ok(paths)
}

/// Rejects paths outside the library, so commands can't touch arbitrary files.
fn in_library(command: &str, s: &Session, path: &Path) -> Result<(), EngineError> {
    if s.catalog
        .folders()
        .iter()
        .any(|root| path.starts_with(root))
    {
        Ok(())
    } else {
        Err(invalid(
            command,
            format!("{} is not in the library", path.display()),
        ))
    }
}

fn category_arg(command: &str, args: &Value) -> Result<CategoryId, EngineError> {
    args.get("id")
        .and_then(Value::as_u64)
        .and_then(|n| CategoryId::try_from(n).ok())
        .ok_or_else(|| invalid(command, "expected {\"id\": number}"))
}

fn known_category(command: &str, s: &Session, id: CategoryId) -> Result<(), EngineError> {
    if s.catalog.categories().iter().any(|c| c.id == id) {
        Ok(())
    } else {
        Err(invalid(command, format!("unknown category {id}")))
    }
}

/// `{"id": number}` or `{"id": null}` for Uncategorized.
fn optional_category(
    command: &str,
    s: &Session,
    args: &Value,
) -> Result<Option<CategoryId>, EngineError> {
    match args.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            let id = category_arg(command, args)?;
            known_category(command, s, id)?;
            Ok(Some(id))
        }
    }
}

fn tag_arg(command: &str, args: &Value) -> Result<String, EngineError> {
    clean_tag(arg_str(command, args, "tag")?).ok_or_else(|| invalid(command, "the tag is empty"))
}

fn color_arg(args: &Value, fallback: [u8; 3]) -> [u8; 3] {
    let Some(parts) = args.get("color").and_then(Value::as_array) else {
        return fallback;
    };
    let channel = |i: usize| {
        parts
            .get(i)
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
    };
    match (channel(0), channel(1), channel(2)) {
        (Some(r), Some(g), Some(b)) => [r, g, b],
        _ => fallback,
    }
}

pub fn register(r: &mut CommandRegistry) {
    register_meta(r);
    register_categories(r);
    register_filters_and_sorting(r);
    register_files(r);
    register_previews(r);
}

fn register_meta(r: &mut CommandRegistry) {
    r.register(Command {
        id: "meta.category",
        label: "Set Category",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "meta.category";
            let category = optional_category(ID, s, args)?;
            let ops = targets(ID, s, args)?
                .into_iter()
                .map(|target| Op::SetCategory { target, category })
                .collect();
            s.apply_all(ops);
            Ok(())
        },
    });
    r.register(Command {
        id: "meta.tag.add",
        label: "Add Tag",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "meta.tag.add";
            let tag = tag_arg(ID, args)?;
            let ops = targets(ID, s, args)?
                .into_iter()
                .map(|target| Op::AddTag {
                    target,
                    tag: tag.clone(),
                })
                .collect();
            s.apply_all(ops);
            Ok(())
        },
    });
    r.register(Command {
        id: "meta.tag.remove",
        label: "Remove Tag",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "meta.tag.remove";
            let tag = tag_arg(ID, args)?;
            let ops = targets(ID, s, args)?
                .into_iter()
                .map(|target| Op::RemoveTag {
                    target,
                    tag: tag.clone(),
                })
                .collect();
            s.apply_all(ops);
            Ok(())
        },
    });
    r.register(Command {
        id: "meta.favorite.toggle",
        label: "Toggle Favorite",
        menu: "Model/Toggle Favorite",
        shortcut: Some(Shortcut::key(Key::Char('h'))),
        enabled: always,
        run: |s, args| {
            const ID: &str = "meta.favorite.toggle";
            let targets = targets(ID, s, args)?;
            // Favorite everything, unless it all is already.
            let favorite = !targets.iter().all(|t| s.catalog.meta_of(t).favorite);
            let ops = targets
                .into_iter()
                .map(|target| Op::SetFavorite { target, favorite })
                .collect();
            s.apply_all(ops);
            Ok(())
        },
    });
}

fn register_categories(r: &mut CommandRegistry) {
    r.register(Command {
        id: "category.add",
        label: "Add Category",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "category.add";
            let name = arg_str(ID, args, "name")?.trim().to_owned();
            if name.is_empty() {
                return Err(invalid(ID, "the name is empty"));
            }
            let fallback = PALETTE
                .get(s.catalog.categories().len() % PALETTE.len())
                .copied()
                .unwrap_or([0x78, 0x90, 0x9c]);
            let category = Category {
                id: s.catalog.new_category_id(),
                name,
                color: color_arg(args, fallback),
            };
            s.apply(Op::AddCategory { category });
            Ok(())
        },
    });
    r.register(Command {
        id: "category.rename",
        label: "Rename Category",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "category.rename";
            let id = category_arg(ID, args)?;
            known_category(ID, s, id)?;
            let name = arg_str(ID, args, "name")?.trim().to_owned();
            if name.is_empty() {
                return Err(invalid(ID, "the name is empty"));
            }
            s.apply(Op::RenameCategory { id, name });
            Ok(())
        },
    });
    r.register(Command {
        id: "category.remove",
        label: "Delete Category",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "category.remove";
            let id = category_arg(ID, args)?;
            known_category(ID, s, id)?;
            s.attributes.categories.retain(|c| *c != Some(id));
            s.apply(Op::RemoveCategory { id });
            Ok(())
        },
    });
}

fn register_filters_and_sorting(r: &mut CommandRegistry) {
    r.register(Command {
        id: "filter.category",
        label: "Filter by Category",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let category = optional_category("filter.category", s, args)?;
            s.attributes.toggle_category(category);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.tag",
        label: "Filter by Tag",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let tag = tag_arg("filter.tag", args)?;
            s.attributes.toggle_tag(tag);
            Ok(())
        },
    });
    r.register(Command {
        id: "filter.favorites",
        label: "Filter by Favorites",
        menu: "Library/Show Only Favorites",
        shortcut: None,
        enabled: always,
        run: |s, _| {
            s.attributes.toggle_favorites();
            Ok(())
        },
    });
    r.register(Command {
        id: "sort.by",
        label: "Sort By",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let name = arg_str("sort.by", args, "key")?;
            let key = SortKey::from_id(name)
                .ok_or_else(|| invalid("sort.by", format!("unknown key \"{name}\"")))?;
            s.sort.choose(key);
            Ok(())
        },
    });
    r.register(Command {
        id: "sort.direction",
        label: "Reverse Sort Order",
        menu: "View/Reverse Sort Order",
        shortcut: None,
        enabled: always,
        run: |s, _| {
            s.sort.dir = s.sort.dir.flipped();
            Ok(())
        },
    });
}

fn register_files(r: &mut CommandRegistry) {
    r.register(Command {
        id: "select.folder",
        label: "Select Folder",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "select.folder";
            let path = PathBuf::from(arg_str(ID, args, "path")?);
            if !s.visible_folders().contains(&path) {
                return Err(invalid(ID, format!("{} is not shown", path.display())));
            }
            s.selection.clear();
            s.current = None;
            s.anchor = None;
            s.focused_folder = Some(path);
            Ok(())
        },
    });
    r.register(Command {
        id: "file.reveal",
        label: "Reveal in File Explorer",
        menu: "Model/Reveal in File Explorer",
        shortcut: Some(Shortcut::ctrl_shift(Key::Char('r'))),
        enabled: always,
        run: |s, args| {
            if let Some(path) = paths("file.reveal", s, args)?.into_iter().next() {
                s.request(Request::RevealFile(path));
            }
            Ok(())
        },
    });
    r.register(Command {
        id: "file.open",
        label: "Open",
        menu: "Model/Open",
        shortcut: Some(Shortcut::ctrl(Key::Char('o'))),
        enabled: always,
        run: |s, args| {
            let first = targets("file.open", s, args)?.into_iter().next();
            match first {
                Some(Target::Folder(path)) => s.request(Request::ShowInExplorer(path)),
                Some(Target::Model(id)) => {
                    if let Some(m) = s.model(id) {
                        let path = m.path.clone();
                        s.request(Request::OpenFile(path));
                    }
                }
                None => {}
            }
            Ok(())
        },
    });
    r.register(Command {
        id: "file.delete",
        label: "Delete…",
        menu: "Model/Delete…",
        shortcut: Some(Shortcut::key(Key::Delete)),
        enabled: always,
        run: |s, args| {
            let paths = paths("file.delete", s, args)?;
            s.request(Request::ConfirmDelete(paths));
            Ok(())
        },
    });
    r.register(Command {
        id: "file.delete_confirmed",
        label: "Move to Recycle Bin",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "file.delete_confirmed";
            let list = args
                .get("paths")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid(ID, "expected {\"paths\": [string]}"))?;
            let paths: Vec<PathBuf> = list
                .iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect();
            for path in &paths {
                in_library(ID, s, path)?;
            }
            s.delete_paths(&paths);
            Ok(())
        },
    });
    r.register(Command {
        id: "library.remove_folder",
        label: "Remove Folder from Library…",
        menu: "Library/Remove Folder from Library…",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "library.remove_folder";
            let path = folder(ID, s, args)?;
            in_library(ID, s, &path)?;
            s.request(Request::ConfirmRemoveFolder(path));
            Ok(())
        },
    });
    r.register(Command {
        id: "library.remove_folder_confirmed",
        label: "Remove Folder",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "library.remove_folder_confirmed";
            let path = PathBuf::from(arg_str(ID, args, "path")?);
            in_library(ID, s, &path)?;
            s.remove_folder(&path);
            Ok(())
        },
    });
    r.register(Command {
        id: "app.settings",
        label: "Settings…",
        menu: "Edit/Settings…",
        shortcut: Some(Shortcut::ctrl(Key::Char(','))),
        enabled: always,
        run: |s, _| {
            s.request(Request::ShowSettings);
            Ok(())
        },
    });
}

fn register_previews(r: &mut CommandRegistry) {
    r.register(Command {
        id: "preview.regenerate",
        label: "Regenerate Thumbnail",
        menu: "Model/Regenerate Thumbnail",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            let mut ids = Vec::new();
            for target in targets("preview.regenerate", s, args)? {
                match target {
                    Target::Model(id) => ids.push(id),
                    // A folder regenerates everything inside it.
                    Target::Folder(dir) => ids.extend(
                        s.catalog
                            .models()
                            .iter()
                            .filter(|m| m.path.starts_with(&dir))
                            .map(|m| m.id),
                    ),
                }
            }
            s.regenerate_thumbnails(&ids);
            Ok(())
        },
    });
    r.register(Command {
        id: "preview.regenerate_all",
        label: "Regenerate All Thumbnails",
        menu: "Library/Regenerate All Thumbnails",
        shortcut: None,
        enabled: always,
        run: |s, _| {
            let mut settings = s.preview_settings();
            settings.generation = settings.generation.wrapping_add(1);
            s.set_preview_settings(settings);
            s.set_status("Regenerating all thumbnails…");
            Ok(())
        },
    });
    r.register(Command {
        id: "preview.settings",
        label: "Preview Settings",
        menu: "",
        shortcut: None,
        enabled: always,
        run: |s, args| {
            const ID: &str = "preview.settings";
            let mut settings = s.preview_settings();
            if let Some(size) = args.get("size") {
                let size = size
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| THUMB_SIZES.contains(n))
                    .ok_or_else(|| invalid(ID, format!("size must be one of {THUMB_SIZES:?}")))?;
                settings.size = size;
            }
            match args.get("background") {
                None => {}
                Some(Value::Null) => settings.background = None,
                Some(v) => settings.background = Some(rgb(ID, "background", v)?),
            }
            if let Some(v) = args.get("model_color") {
                settings.model_color = rgb(ID, "model_color", v)?;
            }
            s.set_preview_settings(settings);
            Ok(())
        },
    });
}

/// `[r, g, b]`.
fn rgb(command: &str, name: &str, value: &Value) -> Result<[u8; 3], EngineError> {
    let channels: Vec<u8> = value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default();
    match channels.as_slice() {
        [r, g, b] => Ok([*r, *g, *b]),
        _ => Err(invalid(command, format!("{name} must be [r, g, b]"))),
    }
}

/// `{"path"}`, or the folder being browsed.
fn folder(command: &str, s: &Session, args: &Value) -> Result<PathBuf, EngineError> {
    crate::library::folder_arg(command, s, args)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use polyroom_catalog::ScannedFile;
    use polyroom_formats::Format;
    use serde_json::json;

    use super::*;
    use crate::{Collection, default_registry};

    /// Two folders under C:/m, with a.stl in `minis` and b.stl at the top.
    fn session() -> Session {
        let mut s = Session::new();
        let scanned = |dir: &str, name: &str, size: u64| ScannedFile {
            path: Path::new(dir).join(name),
            format: Format::Stl,
            size,
            modified: Some(1),
        };
        s.catalog
            .import(
                Path::new("C:/m"),
                vec![
                    scanned("C:/m/minis", "a.stl", 300),
                    scanned("C:/m/terrain", "t.stl", 200),
                    scanned("C:/m", "b.stl", 100),
                ],
            )
            .unwrap();
        s.collection = Collection::Folder("C:/m".into());
        s.refresh_visible();
        s
    }

    fn run(s: &mut Session, id: &str, args: Value) -> Result<(), EngineError> {
        default_registry().run(id, s, &args)
    }

    fn all(s: &Session, pred: impl Fn(&polyroom_catalog::Model) -> bool) -> bool {
        s.catalog().models().iter().all(pred)
    }

    fn names(s: &Session) -> Vec<String> {
        s.visible()
            .iter()
            .filter_map(|&id| s.model(id))
            .map(|m| m.name())
            .collect()
    }

    #[test]
    fn categories_are_created_assigned_and_filtered() {
        let mut s = session();
        run(&mut s, "category.add", json!({"name": " Art "})).unwrap();
        let art = s.catalog().categories().last().unwrap().clone();
        assert_eq!(art.name, "Art");

        // Categorise a folder; the models in it inherit that when filtering.
        run(
            &mut s,
            "meta.category",
            json!({"id": art.id, "path": "C:/m/minis"}),
        )
        .unwrap();
        run(&mut s, "filter.category", json!({"id": art.id})).unwrap();
        s.refresh_visible();
        assert_eq!(names(&s), ["a.stl"]);

        // Uncategorized is an alternative.
        run(&mut s, "filter.category", json!({"id": null})).unwrap();
        s.refresh_visible();
        assert_eq!(names(&s), ["a.stl", "b.stl", "t.stl"]);

        // Deleting the category removes the filter choice and the assignment.
        run(&mut s, "category.remove", json!({"id": art.id})).unwrap();
        assert_eq!(s.attributes().categories, [None]);
        assert!(s.catalog().folder_meta(Path::new("C:/m/minis")).is_none());

        assert!(run(&mut s, "meta.category", json!({"id": 999, "path": "C:/m"})).is_err());
        assert!(run(&mut s, "category.add", json!({"name": "  "})).is_err());
    }

    #[test]
    fn tags_and_favorites_apply_to_the_selection() {
        let mut s = session();
        s.collection = Collection::All;
        s.refresh_visible();
        run(&mut s, "select.all", json!({})).unwrap();
        run(&mut s, "meta.tag.add", json!({"tag": " Dragon "})).unwrap();
        assert!(all(&s, |m| m.meta.tags.contains("dragon")));

        run(&mut s, "meta.tag.remove", json!({"tag": "dragon"})).unwrap();
        assert!(all(&s, |m| m.meta.tags.is_empty()));
        assert!(run(&mut s, "meta.tag.add", json!({"tag": " "})).is_err());

        // Mixed favorites become all favorites; toggling again clears them.
        let first = s.visible()[0];
        s.selection = [first].into_iter().collect();
        run(&mut s, "meta.favorite.toggle", json!({})).unwrap();
        run(&mut s, "select.all", json!({})).unwrap();
        run(&mut s, "meta.favorite.toggle", json!({})).unwrap();
        assert!(all(&s, |m| m.meta.favorite));
        run(&mut s, "meta.favorite.toggle", json!({})).unwrap();
        assert!(all(&s, |m| !m.meta.favorite));

        run(&mut s, "filter.favorites", json!({})).unwrap();
        s.refresh_visible();
        assert!(names(&s).is_empty());
    }

    #[test]
    fn edits_without_a_target_say_so() {
        let mut s = session();
        s.selection.clear();
        s.current = None;
        let err = run(&mut s, "meta.favorite.toggle", json!({})).unwrap_err();
        assert!(err.to_string().contains("select a file or folder"));
    }

    #[test]
    fn sorting_changes_the_grid_order() {
        let mut s = session();
        assert_eq!(names(&s), ["b.stl"], "a folder shows its own files");
        run(&mut s, "filter.favorites", json!({})).unwrap(); // searching shows the subtree
        run(&mut s, "filter.favorites", json!({})).unwrap();
        s.filter = "stl".into();
        s.refresh_visible();
        assert_eq!(names(&s), ["a.stl", "b.stl", "t.stl"]);

        run(&mut s, "sort.by", json!({"key": "size"})).unwrap();
        s.refresh_visible();
        assert_eq!(names(&s), ["a.stl", "t.stl", "b.stl"], "biggest first");
        run(&mut s, "sort.direction", json!({})).unwrap();
        s.refresh_visible();
        assert_eq!(names(&s), ["b.stl", "t.stl", "a.stl"]);
        assert!(run(&mut s, "sort.by", json!({"key": "colour"})).is_err());
    }

    #[test]
    fn a_clicked_folder_is_focused_until_a_model_is_selected() {
        let mut s = session();
        assert_eq!(
            s.visible_folders(),
            [PathBuf::from("C:/m/minis"), PathBuf::from("C:/m/terrain")]
        );
        run(&mut s, "select.folder", json!({"path": "C:/m/minis"})).unwrap();
        assert_eq!(s.focused_folder(), Some(Path::new("C:/m/minis")));
        assert!(s.current().is_none());

        // Folder edits go to the focused folder.
        run(&mut s, "meta.favorite.toggle", json!({})).unwrap();
        assert!(
            s.catalog()
                .folder_meta(Path::new("C:/m/minis"))
                .unwrap()
                .favorite
        );

        let id = s.visible()[0];
        run(&mut s, "select.model", json!({"id": id})).unwrap();
        assert!(s.focused_folder().is_none());
        assert!(run(&mut s, "select.folder", json!({"path": "C:/elsewhere"})).is_err());
    }

    #[test]
    fn delete_asks_first_then_moves_files_and_forgets_them() {
        let dir = tempfile::tempdir().unwrap();
        let (keep, gone) = (dir.path().join("keep.stl"), dir.path().join("gone.stl"));
        std::fs::write(&keep, b"x").unwrap();
        std::fs::write(&gone, b"x").unwrap();
        let mut s = Session::new();
        s.trash = Some(|p| std::fs::remove_file(p).map_err(|e| e.to_string()));
        let scanned = |path: &Path| ScannedFile {
            path: path.to_owned(),
            format: Format::Stl,
            size: 1,
            modified: None,
        };
        s.catalog
            .import(dir.path(), vec![scanned(&keep), scanned(&gone)])
            .unwrap();
        s.collection = Collection::Folder(dir.path().to_owned());
        s.refresh_visible();
        let id = s
            .visible()
            .iter()
            .copied()
            .find(|&i| s.model(i).is_some_and(|m| m.path == gone))
            .unwrap();
        run(&mut s, "select.model", json!({"id": id})).unwrap();

        run(&mut s, "file.delete", json!({})).unwrap();
        assert_eq!(
            s.drain_requests(),
            [Request::ConfirmDelete(vec![gone.clone()])]
        );
        assert!(gone.exists(), "nothing is deleted before the user confirms");
        assert_eq!(s.catalog().models().len(), 2);

        run(
            &mut s,
            "file.delete_confirmed",
            json!({"paths": [gone.to_string_lossy()]}),
        )
        .unwrap();
        assert!(!gone.exists() && keep.exists());
        assert_eq!(s.catalog().models().len(), 1);
        assert_eq!(names(&s), ["keep.stl"]);
        assert!(s.status().contains("Recycle Bin"));
    }

    #[test]
    fn delete_refuses_paths_outside_the_library_and_reports_failures() {
        let mut s = session();
        let outside = run(
            &mut s,
            "file.delete_confirmed",
            json!({"paths": ["C:/Windows/notepad.exe"]}),
        );
        assert!(outside.is_err());

        // A file that cannot be moved stays in the library.
        s.trash = Some(|_| Err("in use".into()));
        run(
            &mut s,
            "file.delete_confirmed",
            json!({"paths": ["C:/m/b.stl"]}),
        )
        .unwrap();
        assert_eq!(s.catalog().models().len(), 3);
        assert!(s.status().contains("in use"), "{}", s.status());
    }

    #[test]
    fn removing_a_folder_asks_first_and_leaves_the_files_alone() {
        let mut s = session();
        s.collection = Collection::Folder("C:/m/minis".into());
        s.refresh_visible();
        run(&mut s, "library.remove_folder", json!({})).unwrap();
        assert_eq!(
            s.drain_requests(),
            [Request::ConfirmRemoveFolder("C:/m/minis".into())]
        );
        assert_eq!(s.catalog().models().len(), 3);

        run(
            &mut s,
            "library.remove_folder_confirmed",
            json!({"path": "C:/m/minis"}),
        )
        .unwrap();
        assert_eq!(s.catalog().models().len(), 2);
        // The folder being viewed is gone, so the view moves to its parent.
        assert_eq!(s.collection(), &Collection::Folder("C:/m".into()));
        assert!(run(&mut s, "library.remove_folder", json!({"path": "D:/x"})).is_err());
    }

    #[test]
    fn file_actions_queue_requests_for_the_front_end() {
        let mut s = session();
        let id = s.visible()[0];
        run(&mut s, "select.model", json!({"id": id})).unwrap();
        run(&mut s, "file.reveal", json!({})).unwrap();
        run(&mut s, "file.open", json!({})).unwrap();
        run(&mut s, "file.reveal", json!({"path": "C:/m/minis"})).unwrap();
        run(&mut s, "app.settings", json!({})).unwrap();
        let b = PathBuf::from("C:/m/b.stl");
        assert_eq!(
            s.drain_requests(),
            [
                Request::RevealFile(b.clone()),
                Request::OpenFile(b),
                Request::RevealFile("C:/m/minis".into()),
                Request::ShowSettings,
            ]
        );
    }

    #[test]
    fn the_left_panel_counts_follow_edits() {
        let mut s = session();
        assert_eq!(s.category_count(None), 3);
        run(&mut s, "category.add", json!({"name": "Art"})).unwrap();
        let art = s.catalog().categories().last().unwrap().id;
        run(
            &mut s,
            "meta.category",
            json!({"id": art, "path": "C:/m/minis"}),
        )
        .unwrap();
        run(
            &mut s,
            "meta.tag.add",
            json!({"tag": "orc", "path": "C:/m/minis"}),
        )
        .unwrap();
        s.refresh_visible();
        assert_eq!(s.category_count(Some(art)), 1);
        assert_eq!(s.category_count(None), 2);
        assert_eq!(s.tag_counts().get("orc"), Some(&1));
        assert_eq!(s.favorite_count(), 0);
        run(&mut s, "meta.favorite.toggle", json!({"path": "C:/m"})).unwrap();
        run(&mut s, "select.all", json!({})).unwrap();
        run(&mut s, "meta.favorite.toggle", json!({})).unwrap();
        s.refresh_visible();
        assert_eq!(
            s.favorite_count(),
            3,
            "a favorite folder makes everything in it a favorite"
        );
    }

    #[test]
    fn the_status_bar_totals_follow_the_library_and_selection() {
        let mut s = session();
        let stats = s.library_stats();
        assert_eq!((stats.folders, stats.files, stats.bytes), (3, 3, 600));
        run(&mut s, "select.all", json!({})).unwrap();
        assert_eq!(s.selection_stats(), (1, 100), "the folder shows one file");
    }

    #[test]
    fn preview_settings_are_validated_applied_and_remembered() {
        let mut s = session();
        let before = s.preview_settings();
        run(
            &mut s,
            "preview.settings",
            json!({"size": 512, "background": [1, 2, 3], "model_color": [255, 0, 0]}),
        )
        .unwrap();
        let now = s.preview_settings();
        assert_eq!(
            (now.size, now.background, now.model_color),
            (512, Some([1, 2, 3]), [255, 0, 0])
        );
        assert_eq!(now.generation, before.generation, "no purge for a new look");
        assert!(s.settings().preview == now, "saved with the other settings");

        // Everything is re-drawn: the UI is told to drop its cached thumbnails.
        let stale = s.take_stale_thumbnails();
        assert_eq!(stale.len(), 3);

        run(&mut s, "preview.settings", json!({"background": null})).unwrap();
        assert_eq!(s.preview_settings().background, None);
        assert_eq!(s.preview_settings().size, 512, "unmentioned fields stay");

        assert!(run(&mut s, "preview.settings", json!({"size": 300})).is_err());
        assert!(run(&mut s, "preview.settings", json!({"model_color": [1, 2]})).is_err());
        assert!(run(&mut s, "preview.settings", json!({"background": "red"})).is_err());
    }

    #[test]
    fn regenerating_refreshes_the_chosen_thumbnails_or_all_of_them() {
        let mut s = session();
        s.thumbnail_failed.insert(0);
        s.selection = [2].into_iter().collect();
        s.current = Some(2);
        run(&mut s, "preview.regenerate", json!({})).unwrap();
        assert_eq!(s.take_stale_thumbnails(), [2]);

        // A folder regenerates the files inside it.
        run(&mut s, "preview.regenerate", json!({"path": "C:/m/minis"})).unwrap();
        let inside: Vec<_> = s
            .catalog()
            .models()
            .iter()
            .filter(|m| m.path.starts_with("C:/m/minis"))
            .map(|m| m.id)
            .collect();
        assert_eq!(s.take_stale_thumbnails(), inside);

        let generation = s.preview_settings().generation;
        run(&mut s, "preview.regenerate_all", json!({})).unwrap();
        assert_eq!(s.preview_settings().generation, generation + 1);
        assert_eq!(s.take_stale_thumbnails().len(), 3);
        assert!(!s.thumbnail_failed(0), "failures are retried");
    }
}
