use serde_json::Value;

use crate::{EngineError, Session};

/// UI-agnostic key, mapped from the toolkit's key type by the front-end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Plus,
    Minus,
    F5,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Shortcut {
    pub const fn key(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl(key: Key) -> Self {
        Self {
            ctrl: true,
            ..Self::key(key)
        }
    }

    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            ctrl: true,
            shift: true,
            ..Self::key(key)
        }
    }

    /// Human-readable form such as `Ctrl+Shift+I`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        match self.key {
            Key::Char(c) => out.extend(c.to_uppercase()),
            Key::Enter => out.push_str("Enter"),
            Key::Escape => out.push_str("Esc"),
            Key::Backspace => out.push_str("Backspace"),
            Key::Tab => out.push_str("Tab"),
            Key::Left => out.push('←'),
            Key::Right => out.push('→'),
            Key::Up => out.push('↑'),
            Key::Down => out.push('↓'),
            Key::Plus => out.push('+'),
            Key::Minus => out.push('-'),
            Key::F5 => out.push_str("F5"),
            Key::Delete => out.push_str("Delete"),
        }
        out
    }
}

pub type EnabledFn = fn(&Session) -> bool;
pub type RunFn = fn(&mut Session, &Value) -> Result<(), EngineError>;

/// One user action. `menu` is a `/`-separated path such as `"View/Grid"`; empty means the
/// command is not shown in menus.
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub shortcut: Option<Shortcut>,
    pub enabled: EnabledFn,
    pub run: RunFn,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("id", &self.id)
            .field("shortcut", &self.shortcut)
            .finish_non_exhaustive()
    }
}

pub fn always(_: &Session) -> bool {
    true
}

#[derive(Debug, Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    /// Adds a command. A later command with the same id replaces the earlier one.
    pub fn register(&mut self, command: Command) {
        if let Some(existing) = self.commands.iter_mut().find(|c| c.id == command.id) {
            log::warn!("command `{}` registered twice; replacing", command.id);
            *existing = command;
        } else {
            self.commands.push(command);
        }
    }

    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// All commands in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter()
    }

    pub fn find_shortcut(&self, shortcut: Shortcut) -> Option<&Command> {
        self.commands.iter().find(|c| c.shortcut == Some(shortcut))
    }

    pub fn is_enabled(&self, id: &str, session: &Session) -> bool {
        self.get(id).is_some_and(|c| (c.enabled)(session))
    }

    /// Runs a command by id after checking it is enabled.
    pub fn run(&self, id: &str, session: &mut Session, args: &Value) -> Result<(), EngineError> {
        let command = self
            .get(id)
            .ok_or_else(|| EngineError::UnknownCommand(id.to_owned()))?;
        if !(command.enabled)(session) {
            return Err(EngineError::Disabled(id.to_owned()));
        }
        (command.run)(session, args)
    }
}
