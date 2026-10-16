//! `cargo xtask samples <dir>`: writes a folder of generated models in every supported format,
//! plus a few broken files, for trying out the app without a real library.

use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use polyroom_mesh::Mesh;

fn torus(major: f32, minor: f32, seg: u32, ring: u32) -> Mesh {
    let mut positions = Vec::new();
    for i in 0..seg {
        let u = i as f32 / seg as f32 * TAU;
        for j in 0..ring {
            let v = j as f32 / ring as f32 * TAU;
            let r = major + minor * v.cos();
            positions.push([r * u.cos(), r * u.sin(), minor + minor * v.sin()]);
        }
    }
    let mut indices = Vec::new();
    for i in 0..seg {
        for j in 0..ring {
            let a = i * ring + j;
            let b = ((i + 1) % seg) * ring + j;
            let c = ((i + 1) % seg) * ring + (j + 1) % ring;
            let d = i * ring + (j + 1) % ring;
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    Mesh::new(positions, indices)
}

/// A vase-like surface of revolution with a closed base, `profile(t)` giving the radius.
fn lathe(height: f32, seg: u32, rows: u32, profile: impl Fn(f32) -> f32) -> Mesh {
    let mut positions = vec![[0.0, 0.0, 0.0]];
    for k in 0..=rows {
        let t = k as f32 / rows as f32;
        let r = profile(t);
        for i in 0..seg {
            let a = i as f32 / seg as f32 * TAU;
            positions.push([r * a.cos(), r * a.sin(), t * height]);
        }
    }
    let at = |k: u32, i: u32| 1 + k * seg + i % seg;
    let mut indices = Vec::new();
    for i in 0..seg {
        indices.extend_from_slice(&[0, at(0, i + 1), at(0, i)]);
    }
    for k in 0..rows {
        for i in 0..seg {
            indices.extend_from_slice(&[at(k, i), at(k, i + 1), at(k + 1, i + 1)]);
            indices.extend_from_slice(&[at(k, i), at(k + 1, i + 1), at(k + 1, i)]);
        }
    }
    Mesh::new(positions, indices)
}

/// A stepped pyramid made of stacked boxes, written as an OBJ with quads.
fn ziggurat_obj(steps: u32, size: f32) -> String {
    let mut s = String::from("# stepped pyramid\n");
    let mut base = 1;
    for k in 0..steps {
        let half = size / 2.0 * (1.0 - k as f32 / steps as f32);
        let (z0, z1) = (k as f32 * 5.0, (k + 1) as f32 * 5.0);
        for z in [z0, z1] {
            for (x, y) in [(-half, -half), (half, -half), (half, half), (-half, half)] {
                let _ = writeln!(s, "v {x} {y} {z}");
            }
        }
        let f = |a: u32| base + a;
        for quad in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            let _ = writeln!(
                s,
                "f {} {} {} {}",
                f(quad[0]),
                f(quad[1]),
                f(quad[2]),
                f(quad[3])
            );
        }
        base += 8;
    }
    s
}

fn stl_binary(mesh: &Mesh) -> Vec<u8> {
    let mut out = b"PolyRoom sample".to_vec();
    out.resize(80, b' ');
    out.extend_from_slice(&(mesh.triangle_count() as u32).to_le_bytes());
    for t in mesh.triangles() {
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        for c in n
            .to_array()
            .into_iter()
            .chain(t.iter().flat_map(|v| v.to_array()))
        {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn stl_ascii(mesh: &Mesh, name: &str) -> String {
    let mut s = format!("solid {name}\n");
    for t in mesh.triangles() {
        let _ = writeln!(s, "  facet normal 0 0 0\n    outer loop");
        for v in t {
            let _ = writeln!(s, "      vertex {} {} {}", v.x, v.y, v.z);
        }
        let _ = writeln!(s, "    endloop\n  endfacet");
    }
    s + &format!("endsolid {name}\n")
}

fn model_xml(objects: &[(u32, &Mesh)], items: &[(u32, [f32; 3])]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>"#,
    );
    for (id, mesh) in objects {
        let _ = write!(s, r#"<object id="{id}" type="model"><mesh><vertices>"#);
        for p in mesh.positions() {
            let _ = write!(s, r#"<vertex x="{}" y="{}" z="{}"/>"#, p[0], p[1], p[2]);
        }
        s += "</vertices><triangles>";
        for [a, b, c] in mesh.indices().as_chunks::<3>().0 {
            let _ = write!(s, r#"<triangle v1="{}" v2="{}" v3="{}"/>"#, a, b, c);
        }
        s += "</triangles></mesh></object>";
    }
    s += "</resources><build>";
    for (id, [x, y, z]) in items {
        let _ = write!(
            s,
            r#"<item objectid="{id}" transform="1 0 0 0 1 0 0 0 1 {x} {y} {z}"/>"#
        );
    }
    s + "</build></model>"
}

fn three_mf(model: &str) -> Result<Vec<u8>, String> {
    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    const TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in [
        ("[Content_Types].xml", TYPES),
        ("_rels/.rels", RELS),
        ("3D/3dmodel.model", model),
    ] {
        w.start_file(name, opts).map_err(|e| e.to_string())?;
        w.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(w.finish().map_err(|e| e.to_string())?.into_inner())
}

pub fn write_samples(dir: &Path) -> Result<(), String> {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    };

    for i in 0..24u32 {
        let t = i as f32 / 23.0;
        let mesh = torus(20.0 + 10.0 * t, 4.0 + 8.0 * t, 64, 32);
        write(&format!("rings/torus_{:02}.stl", i + 1), &stl_binary(&mesh))?;
    }
    for i in 0..12u32 {
        let bulge = 0.2 + i as f32 * 0.08;
        let mesh = lathe(60.0 + i as f32 * 5.0, 72, 48, |t| {
            14.0 + 10.0 * (t * std::f32::consts::PI).sin() * bulge * 3.0 - 6.0 * t
        });
        write(
            &format!("vases/vase_{:02}.stl", i + 1),
            stl_ascii(&mesh, "vase").as_bytes(),
        )?;
    }
    for steps in 2..8u32 {
        write(
            &format!("buildings/ziggurat_{steps}.obj"),
            ziggurat_obj(steps, 60.0).as_bytes(),
        )?;
    }
    let ring = torus(18.0, 5.0, 48, 24);
    let vase = lathe(40.0, 48, 24, |t| 10.0 - 4.0 * t);
    let plate = model_xml(
        &[(1, &ring), (2, &vase)],
        &[(1, [100.0, 100.0, 0.0]), (2, [150.0, 100.0, 0.0])],
    );
    write("plates/ring_and_vase.3mf", &three_mf(&plate)?)?;

    write("broken/not_really.stl", b"this is not an STL file")?;
    write("broken/corrupt.3mf", b"PK\x03\x04 truncated zip")?;
    write("broken/bad_index.obj", b"v 0 0 0\nf 1 2 3\n")?;
    Ok(())
}
