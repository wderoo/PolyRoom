//! 3MF (core spec + the production extension's multi-file models, as written by Bambu Studio
//! and PrusaSlicer). Colours and materials are ignored.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use glam::{Affine3A, Mat3, Vec3};
use polyroom_mesh::Mesh;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{FormatError, MAX_TRIANGLES};

const FMT: &str = "3MF";
/// Largest single zip entry we are willing to inflate (zip-bomb guard).
const MAX_ENTRY_BYTES: u64 = 2 << 30;
/// Maximum component nesting depth (guards against reference cycles).
const MAX_DEPTH: usize = 16;
const DEFAULT_ROOT: &str = "3D/3dmodel.model";

#[derive(Default)]
struct Object {
    mesh: Option<Mesh>,
    components: Vec<Reference>,
}

struct Reference {
    /// Model part the object lives in; `None` means the referencing part.
    path: Option<String>,
    object: u32,
    transform: Affine3A,
}

#[derive(Default)]
struct Part {
    scale: f32,
    objects: HashMap<u32, Object>,
    build: Vec<Reference>,
}

pub fn parse(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| FormatError::invalid(FMT, format!("not a zip archive: {e}")))?;

    let root = root_part_name(&mut zip).unwrap_or_else(|| DEFAULT_ROOT.to_owned());
    let sliced = zip
        .file_names()
        .flatten()
        .any(|n| n.to_ascii_lowercase().ends_with(".gcode"));

    let mut parts: HashMap<String, Part> = HashMap::new();
    let names: Vec<String> = zip
        .file_names()
        .flatten()
        .filter(|n| n.to_ascii_lowercase().ends_with(".model"))
        .map(|n| n.into_owned())
        .collect();
    let mut triangles = 0usize;
    for name in names {
        let data = read_entry(&mut zip, &name)?;
        let part = parse_part(&data, &mut triangles)?;
        parts.insert(normalize(&name), part);
    }

    let root = normalize(&root);
    let root_part = parts
        .get(&root)
        .or_else(|| parts.get(&normalize(DEFAULT_ROOT)))
        .ok_or_else(|| {
            FormatError::invalid(
                FMT,
                if sliced {
                    "this is a sliced G-code project with no model geometry"
                } else {
                    "no 3D model part found"
                },
            )
        })?;
    let root_name = if parts.contains_key(&root) {
        root
    } else {
        normalize(DEFAULT_ROOT)
    };

    let mut out = Mesh::default();
    if root_part.build.is_empty() {
        // No build section: show every object that has geometry.
        for id in root_part.objects.keys() {
            instantiate(&parts, &root_name, *id, Affine3A::IDENTITY, 0, &mut out);
        }
    } else {
        for item in &root_part.build {
            let path = item.path.as_deref().map_or(root_name.clone(), normalize);
            instantiate(&parts, &path, item.object, item.transform, 0, &mut out);
        }
    }
    if out.is_empty() && sliced {
        // Bambu/Orca ".gcode.3mf" exports carry the sliced toolpaths, not the model.
        return Err(FormatError::invalid(
            FMT,
            "this is a sliced G-code project with no model geometry",
        ));
    }
    let scale = root_part.scale;
    if scale != 1.0 {
        out.transform(&Affine3A::from_scale(Vec3::splat(scale)));
    }
    Ok(out)
}

fn instantiate(
    parts: &HashMap<String, Part>,
    path: &str,
    id: u32,
    transform: Affine3A,
    depth: usize,
    out: &mut Mesh,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(part) = parts.get(path) else {
        return;
    };
    let Some(object) = part.objects.get(&id) else {
        return;
    };
    if let Some(mesh) = &object.mesh {
        // Everything is assembled in the root part's unit and converted to mm at the end;
        // slicers always write sub-parts in the same unit as the root.
        let mut mesh = mesh.clone();
        mesh.transform(&transform);
        out.append(&mesh);
    }
    for c in &object.components {
        let child_path = c.path.as_deref().map_or(path.to_owned(), normalize);
        instantiate(
            parts,
            &child_path,
            c.object,
            transform * c.transform,
            depth + 1,
            out,
        );
    }
}

/// Zip entry names have no leading slash; 3MF references do.
fn normalize(path: &str) -> String {
    path.trim_start_matches('/').to_ascii_lowercase()
}

fn read_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, FormatError> {
    let entry = zip
        .by_name(name)
        .map_err(|e| FormatError::invalid(FMT, format!("{name}: {e}")))?;
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(FormatError::TooLarge);
    }
    let mut data = Vec::new();
    entry
        .take(MAX_ENTRY_BYTES)
        .read_to_end(&mut data)
        .map_err(|e| FormatError::invalid(FMT, format!("{name}: {e}")))?;
    Ok(data)
}

/// The start part named in `_rels/.rels`, if any.
fn root_part_name(zip: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Option<String> {
    let data = read_entry(zip, "_rels/.rels").ok()?;
    let mut reader = Reader::from_reader(data.as_slice());
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "Relationship" => {
                let ty = attr(&e, "Type").unwrap_or_default();
                if ty.ends_with("/3dmodel") {
                    return attr(&e, "Target");
                }
            }
            Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .map(|a| a.value.into_owned())
}

fn attr_f32(e: &BytesStart<'_>, name: &str) -> Option<f32> {
    attr(e, name)?
        .trim()
        .parse()
        .ok()
        .filter(|v: &f32| v.is_finite())
}

fn attr_u32(e: &BytesStart<'_>, name: &str) -> Option<u32> {
    attr(e, name)?.trim().parse().ok()
}

/// Parses a 3MF transform `m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31 m32` (row-vector
/// convention: p' = p·M).
fn parse_transform(s: Option<String>) -> Affine3A {
    let Some(s) = s else {
        return Affine3A::IDENTITY;
    };
    let v: Vec<f32> = s
        .split_ascii_whitespace()
        .filter_map(|w| w.parse().ok())
        .filter(|f: &f32| f.is_finite())
        .collect();
    let [m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32] = v.as_slice() else {
        return Affine3A::IDENTITY;
    };
    Affine3A::from_mat3_translation(
        Mat3::from_cols(
            Vec3::new(*m00, *m01, *m02),
            Vec3::new(*m10, *m11, *m12),
            Vec3::new(*m20, *m21, *m22),
        ),
        Vec3::new(*m30, *m31, *m32),
    )
}

fn unit_scale(unit: &str) -> f32 {
    match unit {
        "micron" => 0.001,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        _ => 1.0, // millimeter, the default
    }
}

fn parse_part(data: &[u8], triangles: &mut usize) -> Result<Part, FormatError> {
    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::new();
    let mut part = Part {
        scale: 1.0,
        ..Part::default()
    };

    let mut object: Option<(u32, Object)> = None;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut in_mesh = false;
    let mut in_build = false;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| FormatError::invalid(FMT, format!("XML error: {e}")))?;
        match event {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                "model" => {
                    if let Some(u) = attr(&e, "unit") {
                        part.scale = unit_scale(&u);
                    }
                }
                "object" => {
                    if let Some(id) = attr_u32(&e, "id") {
                        object = Some((id, Object::default()));
                    }
                }
                "mesh" => {
                    in_mesh = true;
                    positions.clear();
                    indices.clear();
                }
                "vertex" if in_mesh => {
                    let p = ["x", "y", "z"].map(|k| attr_f32(&e, k).unwrap_or(0.0));
                    positions.push(p);
                }
                "triangle" if in_mesh => {
                    let (Some(a), Some(b), Some(c)) =
                        (attr_u32(&e, "v1"), attr_u32(&e, "v2"), attr_u32(&e, "v3"))
                    else {
                        continue;
                    };
                    indices.extend_from_slice(&[a, b, c]);
                    *triangles += 1;
                    if *triangles > MAX_TRIANGLES {
                        return Err(FormatError::TooLarge);
                    }
                }
                "component" => {
                    if let (Some((_, obj)), Some(id)) = (&mut object, attr_u32(&e, "objectid")) {
                        obj.components.push(Reference {
                            path: attr(&e, "path"),
                            object: id,
                            transform: parse_transform(attr(&e, "transform")),
                        });
                    }
                }
                "build" => in_build = true,
                "item" if in_build => {
                    if let Some(id) = attr_u32(&e, "objectid") {
                        part.build.push(Reference {
                            path: attr(&e, "path"),
                            object: id,
                            transform: parse_transform(attr(&e, "transform")),
                        });
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                "mesh" => {
                    in_mesh = false;
                    if let Some((_, obj)) = &mut object {
                        obj.mesh = Some(Mesh::new(
                            std::mem::take(&mut positions),
                            std::mem::take(&mut indices),
                        ));
                    }
                }
                "object" => {
                    if let Some((id, obj)) = object.take() {
                        part.objects.insert(id, obj);
                    }
                }
                "build" => in_build = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(part)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use polyroom_mesh::test_cube;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn mesh_xml(id: u32) -> String {
        let cube = test_cube();
        let mut s = format!("<object id=\"{id}\" type=\"model\"><mesh><vertices>");
        for p in cube.positions() {
            s += &format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", p[0], p[1], p[2]);
        }
        s += "</vertices><triangles>";
        for t in cube.indices().chunks(3) {
            s += &format!(
                "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
                t[0], t[1], t[2]
            );
        }
        s + "</triangles></mesh></object>"
    }

    fn zip_of(files: &[(&str, String)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;

    #[test]
    fn single_object_with_build_transform() {
        let model = format!(
            r#"<?xml version="1.0"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>{}</resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 100 0 5"/></build></model>"#,
            mesh_xml(1)
        );
        let m = parse(&zip_of(&[
            ("_rels/.rels", RELS.into()),
            ("3D/3dmodel.model", model),
        ]))
        .unwrap();
        let s = m.stats().unwrap();
        assert_eq!(s.triangles, 12);
        assert!((s.volume - 1000.0).abs() < 1e-2);
        assert_eq!(s.bounds.min, [100.0, 0.0, 5.0]);
    }

    #[test]
    fn production_extension_components_across_parts() {
        // Bambu-style: root object is an assembly of components living in another part.
        let root = r#"<model unit="centimeter" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/1501"><resources><object id="9" type="model"><components><component p:path="/3D/Objects/object_1.model" objectid="1"/><component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 2 0 0"/></components></object></resources><build><item objectid="9"/></build></model>"#;
        let sub = format!(
            r#"<model unit="centimeter"><resources>{}</resources></model>"#,
            mesh_xml(1)
        );
        let m = parse(&zip_of(&[
            ("_rels/.rels", RELS.into()),
            ("3D/3dmodel.model", root.into()),
            ("3D/Objects/object_1.model", sub),
        ]))
        .unwrap();
        let s = m.stats().unwrap();
        assert_eq!(s.triangles, 24);
        // The cube is 10 cm here; the copy is offset by 2 cm.
        assert_eq!(s.bounds.max[0], 120.0);
    }

    #[test]
    fn component_cycle_terminates() {
        let model = r#"<model><resources><object id="1"><components><component objectid="1"/></components></object></resources><build><item objectid="1"/></build></model>"#;
        let m = parse(&zip_of(&[("3D/3dmodel.model", model.into())])).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn transform_uses_row_vector_convention() {
        // 90° about Z: x → y.
        let t = parse_transform(Some("0 1 0 -1 0 0 0 0 1 0 0 0".into()));
        let p = t.transform_point3(Vec3::X);
        assert!((p - Vec3::Y).length() < 1e-6);
    }
}
