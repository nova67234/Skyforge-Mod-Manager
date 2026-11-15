#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod archive;
mod crash;
mod game;
mod gpu;
mod ipc;
mod mods;
mod needs;
mod nexus;
mod theme;
mod widgets;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // Windows starts a fresh copy for every nxm:// link and opened file; hand those to the
    // copy that's already running instead of opening a second window.
    if ipc::forward(&args) {
        return Ok(());
    }
    let listener = ipc::Listener::bind();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Skyrim Mod Manager")
            .with_app_id("skyrim-mod-manager")
            .with_inner_size([1080.0, 720.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Skyrim Mod Manager",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, args, listener)?))),
    )
}
