//! The metadata filter: show only models with a minimum rating, certain flags, colour labels,
//! file formats, categories, tags or favorites.

use polyroom_catalog::{Catalog, CategoryId, ColorLabel, Flag, Model};
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
    /// `None` stands for Uncategorized.
    pub categories: Vec<Option<CategoryId>>,
    /// Models need every tag chosen.
    pub tags: Vec<String>,
    pub favorites: bool,
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
            || !self.categories.is_empty()
            || !self.tags.is_empty()
            || self.favorites
    }

    pub fn matches(&self, m: &Model) -> bool {
        m.rating >= self.min_rating
            && (self.flags.is_empty() || self.flags.contains(&m.flag))
            && (self.labels.is_empty() || m.label.is_some_and(|l| self.labels.contains(&l)))
            && (self.formats.is_empty() || self.formats.contains(&m.format))
    }

    /// [`AttributeFilter::matches`] plus the choices that depend on the catalog: a model also
    /// has the category and tags of the folders above it.
    pub fn matches_in(&self, m: &Model, catalog: &Catalog) -> bool {
        if !self.matches(m) {
            return false;
        }
        if self.favorites && !catalog.effective_favorite(m) {
            return false;
        }
        if !self.categories.is_empty() && !self.categories.contains(&catalog.effective_category(m))
        {
            return false;
        }
        self.tags.is_empty() || {
            let tags = catalog.effective_tags(m);
            self.tags.iter().all(|t| tags.contains(t))
        }
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

    pub fn toggle_category(&mut self, category: Option<CategoryId>) {
        toggle(&mut self.categories, category);
    }

    pub fn toggle_tag(&mut self, tag: String) {
        toggle(&mut self.tags, tag);
    }

    pub fn toggle_favorites(&mut self) {
        self.favorites = !self.favorites;
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
            meta: Default::default(),
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

    #[test]
    fn category_tag_and_favorite_choices_use_the_folders_above() {
        use polyroom_catalog::{Category, Op, ScannedFile, Target};

        let mut c = Catalog::in_memory();
        let root = PathBuf::from("C:/m");
        let sub = root.join("minis");
        let scanned = |dir: &PathBuf, name: &str| ScannedFile {
            path: dir.join(name),
            format: Format::Stl,
            size: 1,
            modified: None,
        };
        c.import(&root, vec![scanned(&sub, "a.stl"), scanned(&root, "b.stl")])
            .unwrap();
        let id_of = |c: &Catalog, n: &str| c.models().iter().find(|m| m.name() == n).unwrap().id;
        let (a, b) = (id_of(&c, "a.stl"), id_of(&c, "b.stl"));
        c.apply(Op::AddCategory {
            category: Category {
                id: 5,
                name: "Minis".into(),
                color: [0; 3],
            },
        })
        .unwrap();
        c.apply(Op::SetCategory {
            target: Target::Folder(sub.clone()),
            category: Some(5),
        })
        .unwrap();
        c.apply(Op::AddTag {
            target: Target::Folder(sub),
            tag: "orc".into(),
        })
        .unwrap();
        c.apply(Op::SetFavorite {
            target: Target::Model(b),
            favorite: true,
        })
        .unwrap();
        let shown = |f: &AttributeFilter| -> Vec<u64> {
            c.models()
                .iter()
                .filter(|m| f.matches_in(m, &c))
                .map(|m| m.id)
                .collect()
        };

        let mut f = AttributeFilter::default();
        f.toggle_category(Some(5));
        assert!(f.is_active());
        assert_eq!(shown(&f), [a], "a.stl inherits the folder category");
        f.toggle_category(None);
        assert_eq!(shown(&f), [a, b], "Uncategorized is an alternative");
        f.clear();

        f.toggle_tag("orc".into());
        assert_eq!(shown(&f), [a]);
        f.toggle_tag("dragon".into());
        assert!(shown(&f).is_empty(), "every tag is required");
        f.clear();

        f.toggle_favorites();
        assert_eq!(shown(&f), [b]);
        f.toggle_category(Some(5));
        assert!(shown(&f).is_empty(), "choices combine");
    }
}
