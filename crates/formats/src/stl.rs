//! STL, binary and ASCII.

use polyroom_mesh::{Mesh, Welder};

use crate::{FormatError, MAX_TRIANGLES};

const HEADER: usize = 80;
const RECORD: usize = 50;

pub fn parse(bytes: &[u8]) -> Result<Mesh, FormatError> {
    // Many binary files start with "solid" too, so trust the size check first.
    if let Some(count) = binary_triangle_count(bytes) {
        return parse_binary(bytes, count);
    }
    if bytes.trim_ascii_start().starts_with(b"solid") {
        return parse_ascii(bytes);
    }
    // Some exporters write a wrong count; salvage whole records, but only if what we read
    // looks like real geometry. Otherwise this is some other file with an .stl name.
    if bytes.len() >= HEADER + 4 {
        let count = (bytes.len() - HEADER - 4) / RECORD;
        let mesh = parse_binary(bytes, count)?;
        // Mesh::new drops triangles with non-finite or absurd coordinates; random bytes lose
        // most of theirs, a real STL with a bad header loses none.
        if count > 0 && mesh.triangle_count() * 10 >= count * 9 {
            return Ok(mesh);
        }
        return Err(FormatError::invalid(
            "STL",
            "this doesn't look like an STL file",
        ));
    }
    Err(FormatError::invalid("STL", "file is too short"))
}

/// The triangle count if the file size exactly matches a binary STL.
fn binary_triangle_count(bytes: &[u8]) -> Option<usize> {
    let count = u32::from_le_bytes(bytes.get(HEADER..HEADER + 4)?.try_into().ok()?) as usize;
    let expected = count.checked_mul(RECORD)?.checked_add(HEADER + 4)?;
    (expected == bytes.len()).then_some(count)
}

fn parse_binary(bytes: &[u8], count: usize) -> Result<Mesh, FormatError> {
    if count > MAX_TRIANGLES {
        return Err(FormatError::TooLarge);
    }
    let body = bytes.get(HEADER + 4..).unwrap_or_default();
    let mut welder = Welder::with_capacity(count / 2);
    for record in body.as_chunks::<RECORD>().0.iter().take(count) {
        // 12 bytes normal (ignored), 3×12 bytes vertices, 2 bytes attribute.
        for v in 0..3 {
            let start = 12 + v * 12;
            let p = read_vec3(record.get(start..start + 12).unwrap_or_default());
            welder.push(p);
        }
    }
    Ok(welder.finish())
}

fn read_vec3(b: &[u8]) -> [f32; 3] {
    let f = |i: usize| {
        b.get(i..i + 4)
            .and_then(|s| s.try_into().ok())
            .map_or(0.0, f32::from_le_bytes)
    };
    [f(0), f(4), f(8)]
}

fn parse_ascii(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let text = String::from_utf8_lossy(bytes);
    let mut welder = Welder::with_capacity(bytes.len() / 200);
    let mut corners: Vec<[f32; 3]> = Vec::with_capacity(3);
    let mut triangles = 0usize;
    for line in text.lines() {
        let mut words = line.split_ascii_whitespace();
        match words.next() {
            Some("vertex") => {
                let mut p = [0.0f32; 3];
                for c in &mut p {
                    *c = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| FormatError::invalid("STL", "bad vertex line"))?;
                }
                corners.push(p);
            }
            Some("endloop") => {
                // Polygons with more than 3 corners: triangulate as a fan.
                if let Some((&first, rest)) = corners.split_first() {
                    for pair in rest.windows(2) {
                        if let [b, c] = pair {
                            welder.push(first);
                            welder.push(*b);
                            welder.push(*c);
                            triangles += 1;
                        }
                    }
                }
                corners.clear();
                if triangles > MAX_TRIANGLES {
                    return Err(FormatError::TooLarge);
                }
            }
            _ => {}
        }
    }
    Ok(welder.finish())
}

#[cfg(test)]
mod tests {
    use polyroom_mesh::test_cube;

    use super::*;

    fn cube_binary(header: &[u8]) -> Vec<u8> {
        let cube = test_cube();
        let mut out = vec![0u8; HEADER];
        out[..header.len()].copy_from_slice(header);
        out.extend_from_slice(&(cube.triangle_count() as u32).to_le_bytes());
        for t in cube.triangles() {
            out.extend_from_slice(&[0u8; 12]);
            for v in t {
                for c in v.to_array() {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0, 0]);
        }
        out
    }

    #[test]
    fn binary_cube() {
        let m = parse(&cube_binary(b"binary")).unwrap();
        assert_eq!(m.triangle_count(), 12);
        assert_eq!(m.positions().len(), 8);
        assert!((m.stats().unwrap().volume - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn binary_starting_with_solid_is_still_binary() {
        let m = parse(&cube_binary(b"solid fooled you")).unwrap();
        assert_eq!(m.triangle_count(), 12);
    }

    #[test]
    fn ascii_cube() {
        let mut s = String::from("solid cube\n");
        for t in test_cube().triangles() {
            s.push_str("  facet normal 0 0 0\n    outer loop\n");
            for v in t {
                s.push_str(&format!("      vertex {} {} {}\n", v.x, v.y, v.z));
            }
            s.push_str("    endloop\n  endfacet\n");
        }
        s.push_str("endsolid cube\n");
        let m = parse(s.as_bytes()).unwrap();
        assert_eq!(m.triangle_count(), 12);
        assert!((m.stats().unwrap().volume - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn truncated_binary_salvages_whole_records() {
        let mut b = cube_binary(b"x");
        b.truncate(b.len() - 30);
        assert_eq!(parse(&b).unwrap().triangle_count(), 11);
    }

    #[test]
    fn huge_count_in_short_file_is_safe() {
        let mut b = vec![0u8; HEADER];
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.extend_from_slice(&[0u8; 100]);
        assert_eq!(parse(&b).unwrap().triangle_count(), 2);
    }
}
