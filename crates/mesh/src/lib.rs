//! Indexed triangle meshes in millimetres, with bounds and print statistics.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use glam::{Affine3A, Vec3};
use serde::{Deserialize, Serialize};

/// Coordinates beyond ±1 km (in mm) are treated as garbage; no printable model is that big.
pub const MAX_COORD: f32 = 1.0e6;

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn size(&self) -> Vec3 {
        Vec3::from(self.max) - Vec3::from(self.min)
    }

    pub fn center(&self) -> Vec3 {
        (Vec3::from(self.min) + Vec3::from(self.max)) * 0.5
    }

    /// Radius of the bounding sphere around [`Self::center`].
    pub fn radius(&self) -> f32 {
        self.size().length() * 0.5
    }
}

/// Summary numbers shown in the metadata panel and used for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeshStats {
    pub triangles: u64,
    pub vertices: u64,
    pub bounds: Aabb,
    /// Enclosed volume in mm³ (meaningful for closed meshes).
    pub volume: f64,
    /// Surface area in mm².
    pub area: f64,
}

/// An indexed triangle mesh. Every index is guaranteed to be in range of `positions`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Mesh {
    /// Builds a mesh, dropping any trailing incomplete triangle and triangles that use an
    /// out-of-range index or an invalid vertex (non-finite, or beyond [`MAX_COORD`]).
    pub fn new(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        let valid: Vec<bool> = positions
            .iter()
            .map(|p| p.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD))
            .collect();
        let indices = indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| t.iter().all(|&i| valid.get(i as usize) == Some(&true)))
            .flatten()
            .copied()
            .collect();
        Self { positions, indices }
    }

    /// Builds a mesh from an unindexed triangle soup, welding identical vertices.
    pub fn from_triangles(triangles: &[[[f32; 3]; 3]]) -> Self {
        let mut welder = Welder::with_capacity(triangles.len() / 2);
        for tri in triangles {
            for v in tri {
                welder.push(*v);
            }
        }
        welder.finish()
    }

    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Iterates triangles as corner positions.
    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|[a, b, c]| {
                let p = |i: &u32| self.positions.get(*i as usize).map(|&p| Vec3::from(p));
                Some([p(a)?, p(b)?, p(c)?])
            })
    }

    /// Bounds of the vertices used by triangles; `None` for an empty mesh.
    pub fn bounds(&self) -> Option<Aabb> {
        let mut it = self
            .indices
            .iter()
            .filter_map(|&i| self.positions.get(i as usize));
        let first = Vec3::from(*it.next()?);
        let (min, max) = it.fold((first, first), |(lo, hi), &p| {
            let p = Vec3::from(p);
            (lo.min(p), hi.max(p))
        });
        Some(Aabb {
            min: min.into(),
            max: max.into(),
        })
    }

    pub fn stats(&self) -> Option<MeshStats> {
        let bounds = self.bounds()?;
        // Work relative to the centre so f32 cross products stay precise for far-off models.
        let c = bounds.center();
        let (mut volume, mut area) = (0.0f64, 0.0f64);
        for [a, b, d] in self.triangles() {
            let (a, b, d) = (a - c, b - c, d - c);
            volume += f64::from(a.dot(b.cross(d))) / 6.0;
            area += f64::from((b - a).cross(d - a).length()) * 0.5;
        }
        Some(MeshStats {
            triangles: self.triangle_count() as u64,
            vertices: self.positions.len() as u64,
            bounds,
            volume: volume.abs(),
            area,
        })
    }

    /// Applies an affine transform to all vertices.
    pub fn transform(&mut self, m: &Affine3A) {
        for p in &mut self.positions {
            *p = m.transform_point3(Vec3::from(*p)).into();
        }
    }

    /// Appends another mesh. Does nothing if the result would exceed `u32` indices.
    pub fn append(&mut self, other: &Mesh) {
        let Ok(base) = u32::try_from(self.positions.len()) else {
            return;
        };
        if u32::try_from(self.positions.len() + other.positions.len()).is_err() {
            return;
        }
        self.positions.extend_from_slice(&other.positions);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }
}

/// Deduplicates bit-identical positions while building a mesh.
pub struct Welder {
    map: HashMap<[u32; 3], u32, BuildHasherDefault<FastHasher>>,
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Welder {
    pub fn with_capacity(vertices: usize) -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(vertices, Default::default()),
            positions: Vec::with_capacity(vertices),
            indices: Vec::with_capacity(vertices.saturating_mul(6)),
        }
    }

    pub fn push(&mut self, p: [f32; 3]) {
        // -0.0 and 0.0 should weld together.
        let p = p.map(|c| if c == 0.0 { 0.0 } else { c });
        let next = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        let positions = &mut self.positions;
        let index = *self.map.entry(p.map(f32::to_bits)).or_insert_with(|| {
            positions.push(p);
            next
        });
        self.indices.push(index);
    }

    pub fn finish(self) -> Mesh {
        Mesh::new(self.positions, self.indices)
    }
}

/// A small multiplicative hasher; SipHash is needlessly slow for vertex welding.
#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

/// A 10 mm cube from (0,0,0) to (10,10,10) with outward-facing triangles. Handy in tests.
pub fn test_cube() -> Mesh {
    let p = |x: f32, y: f32, z: f32| [x * 10.0, y * 10.0, z * 10.0];
    let positions = vec![
        p(0., 0., 0.),
        p(1., 0., 0.),
        p(1., 1., 0.),
        p(0., 1., 0.),
        p(0., 0., 1.),
        p(1., 0., 1.),
        p(1., 1., 1.),
        p(0., 1., 1.),
    ];
    #[rustfmt::skip]
    let indices = vec![
        0, 2, 1, 0, 3, 2, // bottom
        4, 5, 6, 4, 6, 7, // top
        0, 1, 5, 0, 5, 4, // front
        2, 3, 7, 2, 7, 6, // back
        1, 2, 6, 1, 6, 5, // right
        3, 0, 4, 3, 4, 7, // left
    ];
    Mesh::new(positions, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_stats() {
        let s = test_cube().stats().unwrap();
        assert_eq!(s.triangles, 12);
        assert!((s.volume - 1000.0).abs() < 1e-3);
        assert!((s.area - 600.0).abs() < 1e-3);
        assert_eq!(s.bounds.size(), Vec3::splat(10.0));
    }

    #[test]
    fn welding_dedupes_shared_corners() {
        let cube = test_cube();
        let soup: Vec<[[f32; 3]; 3]> = cube.triangles().map(|t| t.map(Into::into)).collect();
        let welded = Mesh::from_triangles(&soup);
        assert_eq!(welded.positions().len(), 8);
        assert_eq!(welded.stats().unwrap().volume, cube.stats().unwrap().volume);
    }

    #[test]
    fn out_of_range_indices_are_dropped() {
        let m = Mesh::new(vec![[0.0; 3]; 3], vec![0, 1, 2, 0, 1, 99, 0]);
        assert_eq!(m.triangle_count(), 1);
    }

    #[test]
    fn triangles_with_garbage_vertices_are_dropped() {
        let positions = vec![
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [f32::NAN, 0.0, 0.0],
            [3.4e38, 0.0, 0.0],
        ];
        let m = Mesh::new(positions, vec![0, 1, 2, 0, 1, 3, 0, 4, 2]);
        assert_eq!(m.triangle_count(), 1);
        assert_eq!(m.bounds().unwrap().max, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn append_offsets_indices() {
        let mut a = test_cube();
        a.append(&test_cube());
        assert_eq!(a.triangle_count(), 24);
        assert_eq!(a.indices().iter().max(), Some(&15));
    }

    #[test]
    fn empty_mesh_has_no_stats() {
        assert!(Mesh::default().stats().is_none());
    }
}
