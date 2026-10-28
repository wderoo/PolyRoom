//! Built-in commands.

use serde_json::Value;

use crate::command::{Command, CommandRegistry, Key, Shortcut, always};
use crate::{EngineError, Module, Request, Session, ViewMode};

pub fn register_all(r: &mut CommandRegistry) {
    // File
    r.register(Command {
        id: "library.import_folder",
        label: "Import Folder…",
        menu: "File/Import Folder…",
        shortcut: Some(Shortcut::ctrl_shift(Key::Char('i'))),
        enabled: always,
        run: |s, _| {
            s.request(Request::PickImportFolder);
            Ok(())
        },
    });
    r.register(Command {
        id: "app.quit",
        label: "Quit",
        menu: "File/Quit",
        shortcut: Some(Shortcut::ctrl(Key::Char('q'))),
        enabled: always,
        run: |s, _| {
            s.request(Request::Quit);
            Ok(())
        },
    });

    // Modules
    r.register(Command {
        id: "module.library",
        label: "Library",
        menu: "Window/Library",
        shortcut: Some(Shortcut::ctrl(Key::Char('1'))),
        enabled: always,
        run: |s, _| {
            s.module = Module::Library;
            Ok(())
        },
    });
    r.register(Command {
        id: "module.viewer",
        label: "Viewer",
        menu: "Window/Viewer",
        shortcut: Some(Shortcut::ctrl(Key::Char('2'))),
        enabled: always,
        run: |s, _| {
            s.module = Module::Viewer;
            Ok(())
        },
    });

    // View
    r.register(Command {
        id: "view.grid",
        label: "Grid",
        menu: "View/Grid",
        shortcut: Some(Shortcut::key(Key::Char('g'))),
        enabled: always,
        run: |s, _| {
            s.module = Module::Library;
            s.view = ViewMode::Grid;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.loupe",
        label: "Loupe",
        menu: "View/Loupe",
        shortcut: Some(Shortcut::key(Key::Char('e'))),
        enabled: always,
        run: |s, _| {
            s.module = Module::Library;
            s.view = ViewMode::Loupe;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_loupe",
        label: "Toggle Grid / Loupe",
        menu: "",
        shortcut: Some(Shortcut::key(Key::Enter)),
        enabled: always,
        run: |s, _| {
            s.view = match s.view {
                ViewMode::Grid => ViewMode::Loupe,
                ViewMode::Loupe => ViewMode::Grid,
            };
            Ok(())
        },
    });
    r.register(Command {
        id: "view.back_to_grid",
        label: "Back to Grid",
        menu: "",
        shortcut: Some(Shortcut::key(Key::Escape)),
        enabled: |s| s.view == ViewMode::Loupe,
        run: |s, _| {
            s.view = ViewMode::Grid;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_left_panel",
        label: "Left Panel",
        menu: "View/Panels/Left Panel",
        shortcut: Some(Shortcut::key(Key::Char('['))),
        enabled: always,
        run: |s, _| {
            s.panels.left = !s.panels.left;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_right_panel",
        label: "Right Panel",
        menu: "View/Panels/Right Panel",
        shortcut: Some(Shortcut::key(Key::Char(']'))),
        enabled: always,
        run: |s, _| {
            s.panels.right = !s.panels.right;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_filmstrip",
        label: "Filmstrip",
        menu: "View/Panels/Filmstrip",
        shortcut: Some(Shortcut::key(Key::Char('f'))),
        enabled: always,
        run: |s, _| {
            s.panels.filmstrip = !s.panels.filmstrip;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_all_panels",
        label: "All Panels",
        menu: "View/Panels/All Panels",
        shortcut: Some(Shortcut::key(Key::Tab)),
        enabled: always,
        run: |s, _| {
            let show = !(s.panels.left || s.panels.right || s.panels.filmstrip);
            s.panels.left = show;
            s.panels.right = show;
            s.panels.filmstrip = show;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_bed_grid",
        label: "Bed Grid",
        menu: "View/Bed Grid",
        shortcut: Some(Shortcut::key(Key::Char('b'))),
        enabled: always,
        run: |s, _| {
            s.show_bed_grid = !s.show_bed_grid;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.toggle_photo_covers",
        label: "Use Photos as Covers",
        menu: "View/Use Photos as Covers",
        shortcut: None,
        enabled: always,
        run: |s, _| {
            s.photo_covers = !s.photo_covers;
            Ok(())
        },
    });
    r.register(Command {
        id: "view.thumb_size",
        label: "Set Thumbnail Size",
        menu: "",
        shortcut: None,
        enabled: always,
        run: set_thumb_size,
    });
    r.register(Command {
        id: "view.thumb_larger",
        label: "Larger Thumbnails",
        menu: "View/Larger Thumbnails",
        shortcut: Some(Shortcut::key(Key::Plus)),
        enabled: |s| s.view == ViewMode::Grid,
        run: |s, _| {
            s.thumb_size = (s.thumb_size + 32.0).min(Session::THUMB_MAX);
            Ok(())
        },
    });
    r.register(Command {
        id: "view.thumb_smaller",
        label: "Smaller Thumbnails",
        menu: "View/Smaller Thumbnails",
        shortcut: Some(Shortcut::key(Key::Minus)),
        enabled: |s| s.view == ViewMode::Grid,
        run: |s, _| {
            s.thumb_size = (s.thumb_size - 32.0).max(Session::THUMB_MIN);
            Ok(())
        },
    });

    crate::library::register(r);

    // Help
    r.register(Command {
        id: "help.shortcuts",
        label: "Keyboard Shortcuts",
        menu: "Help/Keyboard Shortcuts",
        shortcut: Some(Shortcut::ctrl(Key::Char('/'))),
        enabled: always,
        run: |s, _| {
            s.request(Request::ShowShortcuts);
            Ok(())
        },
    });
}

/// `{"size": <px>}`, clamped to the allowed range.
fn set_thumb_size(s: &mut Session, args: &Value) -> Result<(), EngineError> {
    let size = args
        .get("size")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| EngineError::InvalidArgument {
            command: "view.thumb_size".into(),
            reason: "expected {\"size\": number}".into(),
        })?;
    s.thumb_size = (size as f32).clamp(Session::THUMB_MIN, Session::THUMB_MAX);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{EngineError, Key, Request, Session, Shortcut, ViewMode, default_registry};

    #[test]
    fn ids_and_shortcuts_are_unique() {
        let r = default_registry();
        let cmds: Vec<_> = r.iter().collect();
        for (i, a) in cmds.iter().enumerate() {
            for b in &cmds[i + 1..] {
                assert_ne!(a.id, b.id);
                if a.shortcut.is_some() {
                    assert_ne!(a.shortcut, b.shortcut, "{} vs {}", a.id, b.id);
                }
            }
        }
    }

    #[test]
    fn shortcut_dispatch_switches_view() {
        let r = default_registry();
        let mut s = Session::new();
        let id = r.find_shortcut(Shortcut::key(Key::Char('e'))).unwrap().id;
        r.run(id, &mut s, &json!({})).unwrap();
        assert_eq!(s.view, ViewMode::Loupe);
        r.run("view.back_to_grid", &mut s, &json!({})).unwrap();
        assert_eq!(s.view, ViewMode::Grid);
    }

    #[test]
    fn disabled_and_unknown_commands_error() {
        let r = default_registry();
        let mut s = Session::new();
        assert!(matches!(
            r.run("view.back_to_grid", &mut s, &json!({})),
            Err(EngineError::Disabled(_))
        ));
        assert!(matches!(
            r.run("nope", &mut s, &json!({})),
            Err(EngineError::UnknownCommand(_))
        ));
    }

    #[test]
    fn thumb_size_validates_and_clamps() {
        let r = default_registry();
        let mut s = Session::new();
        r.run("view.thumb_size", &mut s, &json!({"size": 10_000}))
            .unwrap();
        assert_eq!(s.thumb_size, Session::THUMB_MAX);
        assert!(
            r.run("view.thumb_size", &mut s, &json!({"size": "big"}))
                .is_err()
        );
    }

    #[test]
    fn requests_are_deduplicated_and_drained() {
        let r = default_registry();
        let mut s = Session::new();
        r.run("library.import_folder", &mut s, &json!({})).unwrap();
        r.run("library.import_folder", &mut s, &json!({})).unwrap();
        assert_eq!(s.drain_requests(), vec![Request::PickImportFolder]);
        assert!(s.drain_requests().is_empty());
    }

    #[test]
    fn toggle_all_panels_hides_then_shows() {
        let r = default_registry();
        let mut s = Session::new();
        r.run("view.toggle_all_panels", &mut s, &json!({})).unwrap();
        assert!(!s.panels.left && !s.panels.right && !s.panels.filmstrip);
        r.run("view.toggle_all_panels", &mut s, &json!({})).unwrap();
        assert!(s.panels.left && s.panels.right && s.panels.filmstrip);
    }
}
