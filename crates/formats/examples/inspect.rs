//! Loads every model under a folder and prints its stats: `cargo run --release -p
//! polyroom-formats --example inspect -- <folder>`.

use std::path::Path;

fn visit(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path);
        } else if polyroom_formats::Format::from_path(&path).is_some() {
            match polyroom_formats::load(&path) {
                Ok(mesh) => match mesh.stats() {
                    Some(s) => println!(
                        "{:>10} tris  min {:?}  max {:?}  {}",
                        s.triangles,
                        s.bounds.min,
                        s.bounds.max,
                        path.display()
                    ),
                    None => println!("     empty  {}", path.display()),
                },
                Err(e) => println!("     ERROR  {e}  {}", path.display()),
            }
        }
    }
}

fn main() {
    match std::env::args().nth(1) {
        Some(dir) => visit(Path::new(&dir)),
        None => eprintln!("usage: inspect <folder>"),
    }
}
