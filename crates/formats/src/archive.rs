//! Looking inside ZIP archives without extracting them: listing their files and reading one
//! entry into memory, with caps so a hostile archive can't exhaust memory.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use crate::{Format, FormatError};

const FMT: &str = "ZIP";
/// Entries listed at most; the rest of a huge archive is ignored.
pub const MAX_ENTRIES: usize = 10_000;

/// One file inside an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Path inside the archive, with `/` separators.
    pub name: String,
    /// Uncompressed size in bytes, as the archive claims.
    pub size: u64,
    /// What kind of file it is, if PolyRoom knows it.
    pub format: Option<Format>,
}

impl ArchiveEntry {
    /// The file name without the folders inside the archive.
    pub fn file_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

fn open(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>, FormatError> {
    let file = File::open(path)?;
    zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| FormatError::invalid(FMT, format!("not a zip archive: {e}")))
}

/// Lists the files in the archive at `path` (folders are left out), in archive order.
pub fn list(path: &Path) -> Result<Vec<ArchiveEntry>, FormatError> {
    list_from(&mut open(path)?)
}

/// Reads the entry `name` of the archive at `path`, refusing entries larger than `max_bytes`.
pub fn read_entry(path: &Path, name: &str, max_bytes: u64) -> Result<Vec<u8>, FormatError> {
    read_from(&mut open(path)?, name, max_bytes)
}

fn list_from<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<Vec<ArchiveEntry>, FormatError> {
    let mut out = Vec::new();
    for i in 0..zip.len().min(MAX_ENTRIES) {
        // Raw access reads only the header; nothing is decompressed.
        let Ok(entry) = zip.by_index_raw(i) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let Ok(name) = entry.name().map(|n| n.replace('\\', "/")) else {
            continue;
        };
        let format = name
            .rsplit_once('.')
            .and_then(|(_, ext)| Format::from_name(ext));
        out.push(ArchiveEntry {
            name,
            size: entry.size(),
            format,
        });
    }
    Ok(out)
}

fn read_from<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, FormatError> {
    let entry = zip
        .by_name(name)
        .map_err(|e| FormatError::invalid(FMT, format!("{name}: {e}")))?;
    if entry.size() > max_bytes {
        return Err(FormatError::TooLarge);
    }
    let mut data = Vec::new();
    // The size header can lie; never inflate more than the cap.
    entry
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|e| FormatError::invalid(FMT, format!("{name}: {e}")))?;
    if data.len() as u64 > max_bytes {
        return Err(FormatError::TooLarge);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use proptest::prelude::*;

    use super::*;

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        w.add_directory("files/", options).unwrap();
        for (name, data) in files {
            w.start_file(*name, options).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn reader(bytes: Vec<u8>) -> zip::ZipArchive<Cursor<Vec<u8>>> {
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn lists_files_with_their_formats() {
        let bytes = archive(&[
            ("README.txt", b"hi"),
            ("files/Dragon Head.STL", b"solid"),
            ("images/cover.jpg", b"jpg"),
        ]);
        let entries = list_from(&mut reader(bytes)).unwrap();
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.file_name(), e.size, e.format))
            .collect();
        assert_eq!(
            summary,
            [
                ("README.txt", 2, None),
                ("Dragon Head.STL", 5, Some(Format::Stl)),
                ("cover.jpg", 3, Some(Format::Image)),
            ]
        );
    }

    #[test]
    fn reads_an_entry_within_the_cap_only() {
        let bytes = archive(&[("files/a.stl", &[7u8; 100])]);
        assert_eq!(
            read_from(&mut reader(bytes.clone()), "files/a.stl", 100).unwrap(),
            [7u8; 100]
        );
        assert!(matches!(
            read_from(&mut reader(bytes.clone()), "files/a.stl", 99),
            Err(FormatError::TooLarge)
        ));
        assert!(read_from(&mut reader(bytes), "missing.stl", 100).is_err());
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_an_error() {
        let dir = std::env::temp_dir().join(format!("polyroom-archive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.zip");
        std::fs::write(&path, b"definitely not a zip").unwrap();
        assert!(list(&path).is_err());
        assert!(list(&dir.join("missing.zip")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            if let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) {
                let _ = list_from(&mut zip);
                let _ = read_from(&mut zip, "a.stl", 1024);
            }
        }
    }
}
