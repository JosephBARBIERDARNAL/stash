#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("stash")
            .with_app_id("stash")
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "stash",
        options,
        Box::new(|cc| Ok(Box::new(stash::app::StashApp::new(cc)))),
    )
}
