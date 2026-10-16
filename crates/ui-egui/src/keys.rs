//! Maps egui key events to engine shortcuts.

use egui::{Event, Key as EKey, Modifiers};
use polyroom_engine::{Key, Shortcut};

fn map_key(key: EKey) -> Option<Key> {
    let k = match key {
        EKey::Enter => Key::Enter,
        EKey::Escape => Key::Escape,
        EKey::Backspace => Key::Backspace,
        EKey::F5 => Key::F5,
        EKey::Tab => Key::Tab,
        EKey::ArrowLeft => Key::Left,
        EKey::ArrowRight => Key::Right,
        EKey::ArrowUp => Key::Up,
        EKey::ArrowDown => Key::Down,
        EKey::Plus | EKey::Equals => Key::Plus,
        EKey::Minus => Key::Minus,
        EKey::OpenBracket => Key::Char('['),
        EKey::CloseBracket => Key::Char(']'),
        EKey::Slash => Key::Char('/'),
        EKey::Num0 => Key::Char('0'),
        EKey::Num1 => Key::Char('1'),
        EKey::Num2 => Key::Char('2'),
        EKey::Num3 => Key::Char('3'),
        EKey::Num4 => Key::Char('4'),
        EKey::Num5 => Key::Char('5'),
        EKey::Num6 => Key::Char('6'),
        EKey::Num7 => Key::Char('7'),
        EKey::Num8 => Key::Char('8'),
        EKey::Num9 => Key::Char('9'),
        other => {
            // Letters: egui names them "A".."Z".
            let name = other.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Char(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
    };
    Some(k)
}

fn shortcut(key: EKey, modifiers: Modifiers) -> Option<Shortcut> {
    Some(Shortcut {
        key: map_key(key)?,
        ctrl: modifiers.command,
        // Shift is part of `+` on most layouts; ignore it there.
        shift: modifiers.shift && !matches!(key, EKey::Plus | EKey::Equals),
        alt: modifiers.alt,
    })
}

/// Shortcuts pressed this frame, in order. Key repeats are ignored.
pub fn pressed_shortcuts(ctx: &egui::Context) -> Vec<Shortcut> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => shortcut(*key, *modifiers),
                _ => None,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_modifiers_map() {
        let s = shortcut(EKey::I, Modifiers::COMMAND | Modifiers::SHIFT).unwrap();
        assert_eq!(s, Shortcut::ctrl_shift(Key::Char('i')));
        assert_eq!(
            shortcut(EKey::G, Modifiers::NONE),
            Some(Shortcut::key(Key::Char('g')))
        );
        assert_eq!(
            shortcut(EKey::Equals, Modifiers::SHIFT),
            Some(Shortcut::key(Key::Plus))
        );
        assert_eq!(
            shortcut(EKey::F5, Modifiers::NONE),
            Some(Shortcut::key(Key::F5))
        );
        assert_eq!(shortcut(EKey::F12, Modifiers::NONE), None);
    }
}
