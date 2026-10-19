//! The metadata filter: show only models with a minimum rating, certain flags, colour labels
//! or file formats.

use polyroom_catalog::{ColorLabel, Flag, Model};
use polyroom_formats::Format;

/// Choices within one attribute are alternatives (Pick *or* Reject); different attributes must
/// all match. An attribute with nothing chosen doesn't constrain anything.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeFilter {
    /// Models need at least this many stars; 0 allows any.
    pub min_rating: u8,
    pub flags: Vec<Flag>,
    pub labels: Vec<ColorLabel>,
    pub formats: Vec<Format>,
}

/// Adds `value` to `set`, or removes it if it is already there.
fn toggle<T: PartialEq>(set: &mut Vec<T>, value: T) {
    match set.iter().position(|v| *v == value) {
        Some(i) => {
            set.remove(i);
        }
        None => set.push(value),
    }
}

impl AttributeFilter {
    pub fn is_active(&self) -> bool {
        self.min_rating > 0
            || !self.flags.is_empty()
            || !self.labels.is_empty()
            || !self.formats.is_empty()
    }

    pub fn matches(&self, m: &Model) -> bool {
        m.rating >= self.min_rating
            && (self.flags.is_empty() || self.flags.contains(&m.flag))
            && (self.labels.is_empty() || m.label.is_some_and(|l| self.labels.contains(&l)))
            && (self.formats.is_empty() || self.formats.contains(&m.format))
    }

    /// Requires at least `stars` stars (clamped to 5); choosing the current value again, or 0,
    /// removes the requirement.
    pub fn toggle_min_rating(&mut self, stars: u8) {
        let stars = stars.min(5);
        self.min_rating = if stars == self.min_rating { 0 } else { stars };
    }

    pub fn toggle_flag(&mut self, flag: Flag) {
        toggle(&mut self.flags, flag);
    }

    pub fn toggle_label(&mut self, label: ColorLabel) {
        toggle(&mut self.labels, label);
    }

    pub fn toggle_format(&mut self, format: Format) {
        toggle(&mut self.formats, format);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn model(rating: u8, flag: Flag, label: Option<ColorLabel>, format: Format) -> Model {
        Model {
            id: 0,
            path: PathBuf::from("C:/m/a.stl"),
            folder: PathBuf::from("C:/m"),
            format,
            size: 1,
            modified: None,
            added: 0,
            import: 1,
            rating,
            flag,
            label,
            stats: None,
            error: None,
        }
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        let f = AttributeFilter::default();
        assert!(!f.is_active());
        assert!(f.matches(&model(0, Flag::None, None, Format::Stl)));
    }

    #[test]
    fn rating_means_at_least() {
        let mut f = AttributeFilter::default();
        f.toggle_min_rating(3);
        assert!(f.is_active());
        assert!(!f.matches(&model(2, Flag::None, None, Format::Stl)));
        assert!(f.matches(&model(3, Flag::None, None, Format::Stl)));
        assert!(f.matches(&model(5, Flag::None, None, Format::Stl)));
        f.toggle_min_rating(3); // choosing it again clears it
        assert!(!f.is_active());
        f.toggle_min_rating(9); // clamped
        assert_eq!(f.min_rating, 5);
    }

    #[test]
    fn choices_within_an_attribute_are_alternatives_and_attributes_combine() {
        let mut f = AttributeFilter::default();
        f.toggle_flag(Flag::Pick);
        f.toggle_flag(Flag::None);
        assert!(f.matches(&model(0, Flag::Pick, None, Format::Stl)));
        assert!(f.matches(&model(0, Flag::None, None, Format::Stl)));
        assert!(!f.matches(&model(0, Flag::Reject, None, Format::Stl)));

        f.toggle_label(ColorLabel::Red);
        assert!(
            !f.matches(&model(0, Flag::Pick, None, Format::Stl)),
            "needs a label"
        );
        assert!(!f.matches(&model(0, Flag::Pick, Some(ColorLabel::Blue), Format::Stl)));
        assert!(f.matches(&model(0, Flag::Pick, Some(ColorLabel::Red), Format::Stl)));

        f.toggle_format(Format::Obj);
        assert!(!f.matches(&model(0, Flag::Pick, Some(ColorLabel::Red), Format::Stl)));
        assert!(f.matches(&model(0, Flag::Pick, Some(ColorLabel::Red), Format::Obj)));

        f.toggle_flag(Flag::Pick); // un-choose
        assert_eq!(f.flags, [Flag::None]);
        f.clear();
        assert!(!f.is_active());
    }
}
