use eframe::egui;
use flowers::gui::FlowersApp;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("flowers — FlowJo Alternative (GUI)"),
        ..Default::default()
    };

    eframe::run_native(
        "flowers-gui",
        options,
        Box::new(|_cc| Ok(Box::new(FlowersApp::default()))),
    )
}
