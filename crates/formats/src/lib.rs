//! Loaders for 3D-print model formats. All loaders treat their input as hostile: malformed
//! files produce a [`FormatError`], never a panic.

#![forbid(unsafe_code)]

mod obj;
mod stl;
mod threemf;

use std::path::Path;

use polyroom_mesh::Mesh;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard cap on triangles per model, to keep a hostile file from exhausting memory.
pub const MAX_TRIANGLES: usize = 50_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Format {
    Stl,
    Obj,
    ThreeMf,
}

impl Format {
    pub const ALL: [Self; 3] = [Self::Stl, Self::ThreeMf, Self::Obj];

    /// Detects the format from a file extension (case-insensitive).
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "stl" => Some(Self::Stl),
            "obj" => Some(Self::Obj),
            "3mf" => Some(Self::ThreeMf),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::Obj => "OBJ",
            Self::ThreeMf => "3MF",
        }
    }
}

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("unsupported file type")]
    Unsupported,
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid {format} file: {reason}")]
    Invalid {
        format: &'static str,
        reason: String,
    },
    #[error("model has more than {MAX_TRIANGLES} triangles")]
    TooLarge,
    #[error("file contains no triangles")]
    Empty,
}

impl FormatError {
    fn invalid(format: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            format,
            reason: reason.into(),
        }
    }
}

/// Loads a model file into a single mesh in millimetres.
pub fn load(path: &Path) -> Result<Mesh, FormatError> {
    let format = Format::from_path(path).ok_or(FormatError::Unsupported)?;
    let bytes = std::fs::read(path)?;
    load_bytes(format, &bytes)
}

/// Recognises common non-model files by their magic bytes, for a helpful error when one has a
/// model extension.
fn sniff_other(bytes: &[u8]) -> Option<&'static str> {
    const SIGNATURES: &[(&[u8], &str)] = &[
        (b"\x89PNG", "a PNG image"),
        (b"\xFF\xD8\xFF", "a JPEG image"),
        (b"GIF8", "a GIF image"),
        (b"%PDF", "a PDF document"),
        (b"PK\x03\x04", "a ZIP archive"),
        (b"Rar!", "a RAR archive"),
        (b"7z\xBC\xAF", "a 7-Zip archive"),
        (b"ISO-10303-21", "a STEP file"),
    ];
    SIGNATURES
        .iter()
        .find(|(magic, _)| bytes.starts_with(magic))
        .map(|(_, what)| *what)
}

/// Parses an in-memory model file.
pub fn load_bytes(format: Format, bytes: &[u8]) -> Result<Mesh, FormatError> {
    // 3MF files are ZIP archives; everything else must not be one of these.
    if let Some(what) = sniff_other(bytes).filter(|_| format != Format::ThreeMf) {
        return Err(FormatError::invalid(
            format.label(),
            format!(
                "this is actually {what} with a .{} name",
                format.label().to_lowercase()
            ),
        ));
    }
    let mesh = match format {
        Format::Stl => stl::parse(bytes)?,
        Format::Obj => obj::parse(bytes)?,
        Format::ThreeMf => threemf::parse(bytes)?,
    };
    if mesh.is_empty() {
        return Err(FormatError::Empty);
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn misnamed_files_are_reported() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(306_860, 0x5a); // the size of the real-world file that crashed us
        let err = load_bytes(Format::Stl, &png).unwrap_err().to_string();
        assert!(err.contains("PNG image"), "{err}");
    }

    #[test]
    fn random_binary_with_stl_name_is_rejected_not_salvaged() {
        // High-entropy bytes whose length doesn't match the triangle count.
        let mut x: u32 = 12345;
        let bytes: Vec<u8> = (0..300_007)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        assert!(load_bytes(Format::Stl, &bytes).is_err());
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(Format::from_path(Path::new("a/B.STL")), Some(Format::Stl));
        assert_eq!(Format::from_path(Path::new("x.3mf")), Some(Format::ThreeMf));
        assert_eq!(Format::from_path(Path::new("x.step")), None);
        assert_eq!(Format::from_path(Path::new("noext")), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            for f in [Format::Stl, Format::Obj, Format::ThreeMf] {
                let _ = load_bytes(f, &bytes);
            }
        }

        #[test]
        fn random_obj_text_never_panics(text in "([vf] [-0-9 ./]{0,30}\n){0,40}") {
            let _ = load_bytes(Format::Obj, text.as_bytes());
        }
    }
}
