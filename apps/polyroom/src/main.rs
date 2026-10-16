#![forbid(unsafe_code)]
// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use polyroom_ui_egui::PolyRoomApp;

fn main() -> eframe::Result {
    // `polyroom --import <folder>` imports a folder at startup.
    let mut args = std::env::args().skip(1);
    let mut import = None;
    while let Some(arg) = args.next() {
        if arg == "--import" {
            import = args.next().map(std::path::PathBuf::from);
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PolyRoom")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PolyRoom",
        options,
        Box::new(|cc| Ok(Box::new(PolyRoomApp::new(cc, import)))),
    )
}
