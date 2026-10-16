use eframe::egui;
use flowers::gui::FlowersApp;
use std::path::PathBuf;

fn main() -> eframe::Result {
    env_logger::init();

    // Any paths given on the command line are loaded at startup.
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("flowers — FlowJo Alternative (GUI)"),
        ..Default::default()
    };

    eframe::run_native(
        "flowers-gui",
        options,
        Box::new(move |_cc| {
            let mut app = FlowersApp::default();
            app.open_paths(&paths);
            Ok(Box::new(app))
        }),
    )
}
