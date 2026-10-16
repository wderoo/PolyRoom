//! Wavefront OBJ: geometry only (`v` and `f`); materials, normals and UVs are ignored.

use polyroom_mesh::Mesh;

use crate::{FormatError, MAX_TRIANGLES};

pub fn parse(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let text = String::from_utf8_lossy(bytes);
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut face: Vec<u32> = Vec::with_capacity(8);

    for line in text.lines() {
        let mut words = line.split_ascii_whitespace();
        match words.next() {
            Some("v") => {
                let mut p = [0.0f32; 3];
                for c in &mut p {
                    *c = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| FormatError::invalid("OBJ", "bad vertex line"))?;
                }
                positions.push(p);
            }
            Some("f") => {
                face.clear();
                for w in words {
                    // "7", "7/1", "7//3" or "7/1/3"; negative indices count from the end.
                    let raw: i64 = w
                        .split('/')
                        .next()
                        .and_then(|i| i.parse().ok())
                        .ok_or_else(|| FormatError::invalid("OBJ", "bad face index"))?;
                    let count = positions.len() as i64;
                    let index = match raw {
                        r if r > 0 => r - 1,
                        r if r < 0 => count + r,
                        _ => return Err(FormatError::invalid("OBJ", "face index 0")),
                    };
                    if !(0..count).contains(&index) {
                        return Err(FormatError::invalid("OBJ", "face index out of range"));
                    }
                    face.push(u32::try_from(index).map_err(|_| FormatError::TooLarge)?);
                }
                // Triangulate the polygon as a fan.
                if let Some((&first, rest)) = face.split_first() {
                    for pair in rest.windows(2) {
                        if let [b, c] = pair {
                            indices.extend_from_slice(&[first, *b, *c]);
                        }
                    }
                }
                if indices.len() / 3 > MAX_TRIANGLES {
                    return Err(FormatError::TooLarge);
                }
            }
            _ => {}
        }
    }
    Ok(Mesh::new(positions, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE: &str = "\
# unit cube, quads
v 0 0 0
v 10 0 0
v 10 10 0
v 0 10 0
v 0 0 10
v 10 0 10
v 10 10 10
v 0 10 10
f 1 4 3 2
f 5 6 7 8
f 1/1 2/2 6/3 5/4
f 3//1 4//1 8//1 7//1
f 2 3 7 6
f -8 -4 -1 -5
";

    #[test]
    fn quad_cube_is_triangulated() {
        let m = parse(CUBE.as_bytes()).unwrap();
        assert_eq!(m.triangle_count(), 12);
        assert!((m.stats().unwrap().volume - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn bad_index_is_an_error() {
        assert!(parse(b"v 0 0 0\nf 1 2 3\n").is_err());
        assert!(parse(b"v 0 0 0\nf 0 1 1\n").is_err());
    }
}
