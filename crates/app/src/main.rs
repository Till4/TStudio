#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod compile;
mod live;
mod logo;
mod midi;
mod samples;
mod settings;
mod ui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TStudio")
            .with_inner_size([1600.0, 950.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: logo::rgba(64), width: 64, height: 64 })),
        ..Default::default()
    };
    // `tstudio.exe song.tsp` opens a project (used by `tstudio-cli open`)
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| p.extension().map(|e| e == "tsp").unwrap_or(false));
    eframe::run_native("TStudio", options, Box::new(|cc| Ok(Box::new(app::TStudio::new(cc, open)))))
}
