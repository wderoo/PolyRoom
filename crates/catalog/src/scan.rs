use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use polyroom_formats::Format;

/// A file found on disk: a model, image or archive, or any other file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub format: Format,
    pub size: u64,
    pub modified: Option<u64>,
}

/// Recursively finds files under `root`, sorted by path. Unreadable entries are skipped.
pub fn scan_folder(root: &Path) -> Vec<ScannedFile> {
    let mut out: Vec<ScannedFile> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let format = Format::classify(e.path());
            let meta = e.metadata().ok()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            Some(ScannedFile {
                path: e.into_path(),
                format,
                size: meta.len(),
                modified,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_supported_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        for name in [
            "a.stl",
            "b.STL",
            "sub/c.3mf",
            "sub/d.obj",
            "notes.txt",
            "e.step",
            "a.png",
            "sub/c.zip",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let found = scan_folder(dir.path());
        let names: Vec<_> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "a.png",
                "a.stl",
                "b.STL",
                "e.step",
                "notes.txt",
                "c.3mf",
                "c.zip",
                "d.obj"
            ]
        );
        let format_of = |name: &str| {
            found
                .iter()
                .find(|f| f.path.file_name().is_some_and(|n| n == name))
                .map(|f| f.format)
        };
        assert_eq!(format_of("notes.txt"), Some(Format::Other));
        assert_eq!(format_of("b.STL"), Some(Format::Stl));
        assert!(found.iter().all(|f| f.size == 1));
    }

    #[test]
    fn missing_folder_is_empty() {
        assert!(scan_folder(Path::new("Z:/definitely/not/here")).is_empty());
    }
}
