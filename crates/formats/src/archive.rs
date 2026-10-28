//! Looking inside ZIP, RAR and 7z archives without extracting them: listing their files and
//! reading one entry into memory, with caps so a hostile archive can't exhaust memory.

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
    match Format::from_path(path) {
        Some(Format::Rar) => rar::list(path),
        Some(Format::SevenZ) => sevenz::list(path),
        _ => list_from(&mut open(path)?),
    }
}

/// Reads the entry `name` of the archive at `path`, refusing entries larger than `max_bytes`.
pub fn read_entry(path: &Path, name: &str, max_bytes: u64) -> Result<Vec<u8>, FormatError> {
    match Format::from_path(path) {
        Some(Format::Rar) => rar::read_entry(path, name, max_bytes),
        Some(Format::SevenZ) => sevenz::read_entry(path, name, max_bytes),
        _ => read_from(&mut open(path)?, name, max_bytes),
    }
}

/// 7z archives, read from the file without loading all of it.
mod sevenz {
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::Path;

    use sevenz_rust2::{Archive, ArchiveReader, Password};

    use super::{ArchiveEntry, MAX_ENTRIES};
    use crate::{Format, FormatError};

    const FMT: &str = "7z";
    /// The most a solid archive may decode, earlier files included, to reach one entry.
    const MAX_DECODE: u64 = 1 << 30;

    fn normalize(name: &str) -> String {
        name.replace('\\', "/")
    }

    fn invalid(e: impl std::fmt::Display) -> FormatError {
        FormatError::invalid(FMT, format!("not a readable 7z archive: {e}"))
    }

    pub fn list(path: &Path) -> Result<Vec<ArchiveEntry>, FormatError> {
        let archive = Archive::open(path).map_err(invalid)?;
        Ok(archive
            .files
            .iter()
            .filter(|e| !e.is_directory && !e.is_anti_item)
            .take(MAX_ENTRIES)
            .map(|e| {
                let name = normalize(&e.name);
                let format = name
                    .rsplit_once('.')
                    .and_then(|(_, ext)| Format::from_name(ext));
                ArchiveEntry {
                    name,
                    size: e.size,
                    format,
                }
            })
            .collect())
    }

    pub fn read_entry(path: &Path, name: &str, max_bytes: u64) -> Result<Vec<u8>, FormatError> {
        let file = File::open(path)?;
        let mut reader = ArchiveReader::new(file, Password::empty()).map_err(invalid)?;
        let declared = reader
            .archive()
            .files
            .iter()
            .find(|e| !e.is_directory && normalize(&e.name) == name)
            .map(|e| e.size)
            .ok_or_else(|| {
                FormatError::invalid(FMT, format!("{name}: no such file in the archive"))
            })?;
        if declared > max_bytes {
            return Err(FormatError::TooLarge);
        }
        let mut data = Vec::new();
        let mut budget = MAX_DECODE.max(max_bytes);
        let mut failure = None;
        // Solid archives must be decoded up to the entry; the budget bounds that work, and the
        // entry itself is never read past the cap even if its header lies about its size.
        let result = reader.for_each_entries(|entry, content| {
            let wanted = !entry.is_directory && normalize(&entry.name) == name;
            let limit = if wanted {
                max_bytes.saturating_add(1)
            } else {
                budget
            };
            let mut taken = content.take(limit);
            if wanted {
                if let Err(e) = taken.read_to_end(&mut data) {
                    failure = Some(e);
                }
                return Ok(false);
            }
            let used = io::copy(&mut taken, &mut io::sink())?;
            budget = budget.saturating_sub(used);
            if budget == 0 {
                failure = Some(io::Error::other("too much data to decode before the file"));
                return Ok(false);
            }
            Ok(true)
        });
        result.map_err(invalid)?;
        if let Some(e) = failure {
            return Err(FormatError::invalid(FMT, format!("{name}: {e}")));
        }
        if data.len() as u64 > max_bytes {
            return Err(FormatError::TooLarge);
        }
        if data.is_empty() && declared > 0 {
            return Err(FormatError::invalid(
                FMT,
                format!("{name}: could not be read"),
            ));
        }
        Ok(data)
    }
}

/// RAR archives, read from the file without loading all of it.
mod rar {
    use std::fs::File;
    use std::path::Path;

    use rars::{ArchiveReadOptions, ArchiveReader};

    use super::{ArchiveEntry, MAX_ENTRIES};
    use crate::{Format, FormatError};

    const FMT: &str = "RAR";
    /// Headers read at most; far more than [`MAX_ENTRIES`] so directories don't use them up.
    const MAX_HEADERS: u64 = 100_000;
    /// The most a solid archive may decode, earlier members included, to reach one entry.
    const MAX_DECODE: u64 = 1 << 30;

    fn open(path: &Path, options: ArchiveReadOptions<'_>) -> Result<rars::Archive, FormatError> {
        let file = File::open(path)?;
        ArchiveReader::read_reader_with_options(file, options)
            .map_err(|e| FormatError::invalid(FMT, format!("not a readable rar archive: {e}")))
    }

    fn normalize(name: &[u8]) -> String {
        String::from_utf8_lossy(name).replace('\\', "/")
    }

    pub fn list(path: &Path) -> Result<Vec<ArchiveEntry>, FormatError> {
        let archive = open(
            path,
            ArchiveReadOptions::new().with_max_header_count(MAX_HEADERS),
        )?;
        Ok(archive
            .members()
            .filter(|m| !m.meta.is_directory && !m.meta.is_redirection)
            .take(MAX_ENTRIES)
            .map(|m| {
                let name = normalize(m.meta.name_bytes());
                let format = name
                    .rsplit_once('.')
                    .and_then(|(_, ext)| Format::from_name(ext));
                ArchiveEntry {
                    name,
                    size: m.meta.unpacked_size,
                    format,
                }
            })
            .collect())
    }

    pub fn read_entry(path: &Path, name: &str, max_bytes: u64) -> Result<Vec<u8>, FormatError> {
        let archive = open(
            path,
            ArchiveReadOptions::new().with_max_header_count(MAX_HEADERS),
        )?;
        let missing = || FormatError::invalid(FMT, format!("{name}: no such file in the archive"));
        let index = archive
            .member_refs()
            .position(|m| {
                !m.is_directory() && !m.is_redirection() && normalize(m.name_bytes()) == name
            })
            .ok_or_else(missing)?;
        let size = archive
            .members()
            .find(|m| !m.meta.is_directory && normalize(m.meta.name_bytes()) == name)
            .map(|m| m.meta.unpacked_size)
            .ok_or_else(missing)?;
        if size > max_bytes {
            return Err(FormatError::TooLarge);
        }
        let options = ArchiveReadOptions::new()
            .with_max_member_output_bytes(max_bytes)
            .with_max_total_output_bytes(MAX_DECODE.max(max_bytes));
        let data = archive
            .read_member_at_with_options(index, options)
            .map_err(|e| FormatError::invalid(FMT, format!("{name}: {e}")))?
            .ok_or_else(missing)?;
        if data.len() as u64 > max_bytes {
            return Err(FormatError::TooLarge);
        }
        Ok(data)
    }
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

    /// A RAR file in a temp folder holding `files`.
    fn rar_file(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let mut builder = rars::Builder::new(rars::ArchiveVersion::Rar50);
        for (name, data) in files {
            builder
                .add_bytes(name.as_bytes().to_vec(), data.to_vec(), None, None)
                .unwrap();
        }
        let path = dir.join("pack.rar");
        std::fs::write(&path, builder.to_bytes().unwrap()).unwrap();
        path
    }

    #[test]
    fn lists_and_reads_the_files_of_a_rar_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = rar_file(
            dir.path(),
            &[
                ("README.txt", b"hi"),
                ("files/Dragon Head.STL", &[7u8; 100]),
                ("images/cover.jpg", b"jpg"),
            ],
        );
        let entries = list(&path).unwrap();
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.file_name(), e.size, e.format))
            .collect();
        assert_eq!(
            summary,
            [
                ("README.txt", 2, None),
                ("Dragon Head.STL", 100, Some(Format::Stl)),
                ("cover.jpg", 3, Some(Format::Image)),
            ]
        );
        assert_eq!(
            read_entry(&path, "files/Dragon Head.STL", 100).unwrap(),
            [7u8; 100]
        );
        assert!(matches!(
            read_entry(&path, "files/Dragon Head.STL", 99),
            Err(FormatError::TooLarge)
        ));
        assert!(read_entry(&path, "missing.stl", 100).is_err());
    }

    #[test]
    fn damaged_and_empty_rar_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let good = rar_file(dir.path(), &[("a.stl", &[1u8; 5000])]);
        let bytes = std::fs::read(&good).unwrap();
        let bad = dir.path().join("bad.rar");
        std::fs::write(&bad, b"not a rar at all").unwrap();
        assert!(list(&bad).is_err());
        std::fs::write(&bad, b"").unwrap();
        assert!(list(&bad).is_err());
        // Every cut-off and every flipped byte must be handled.
        for cut in (0..bytes.len()).step_by(7) {
            std::fs::write(&bad, &bytes[..cut]).unwrap();
            let _ = list(&bad);
            let _ = read_entry(&bad, "a.stl", 1 << 20);
        }
        for at in (0..bytes.len()).step_by(11) {
            let mut damaged = bytes.clone();
            damaged[at] ^= 0xff;
            std::fs::write(&bad, &damaged).unwrap();
            let _ = list(&bad);
            let _ = read_entry(&bad, "a.stl", 1 << 20);
        }
    }

    /// A 7z file in a temp folder holding `files`.
    fn sevenz_file(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let source = dir.join("src");
        for (name, data) in files {
            let path = source.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }
        let out = dir.join("pack.7z");
        sevenz_rust2::compress_to_path(&source, &out).unwrap();
        out
    }

    #[test]
    fn lists_and_reads_the_files_of_a_7z_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = sevenz_file(
            dir.path(),
            &[
                ("README.txt", b"hi"),
                ("files/Dragon Head.STL", &[7u8; 100]),
                ("images/cover.jpg", b"jpg"),
            ],
        );
        let mut entries = list(&path).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.file_name(), e.size, e.format))
            .collect();
        assert_eq!(
            summary,
            [
                ("README.txt", 2, None),
                ("Dragon Head.STL", 100, Some(Format::Stl)),
                ("cover.jpg", 3, Some(Format::Image)),
            ]
        );
        assert_eq!(
            read_entry(&path, "files/Dragon Head.STL", 100).unwrap(),
            [7u8; 100]
        );
        assert!(matches!(
            read_entry(&path, "files/Dragon Head.STL", 99),
            Err(FormatError::TooLarge)
        ));
        assert!(read_entry(&path, "missing.stl", 100).is_err());
    }

    #[test]
    fn damaged_and_empty_7z_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let good = sevenz_file(dir.path(), &[("a.stl", &[1u8; 5000])]);
        let bytes = std::fs::read(&good).unwrap();
        let bad = dir.path().join("bad.7z");
        std::fs::write(&bad, b"not a 7z at all").unwrap();
        assert!(list(&bad).is_err());
        std::fs::write(&bad, b"").unwrap();
        assert!(list(&bad).is_err());
        // Every cut-off and every flipped byte must be handled.
        for cut in (0..bytes.len()).step_by(5) {
            std::fs::write(&bad, &bytes[..cut]).unwrap();
            let _ = list(&bad);
            let _ = read_entry(&bad, "a.stl", 1 << 20);
        }
        for at in (0..bytes.len()).step_by(5) {
            let mut damaged = bytes.clone();
            damaged[at] ^= 0xff;
            std::fs::write(&bad, &damaged).unwrap();
            let _ = list(&bad);
            let _ = read_entry(&bad, "a.stl", 1 << 20);
        }
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
