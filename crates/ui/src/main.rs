use xdj_ui::app::{DeckApp, Options};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(if std::env::args().any(|a| a == "--help") {
                0
            } else {
                2
            });
        }
    };
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(if options.view == "player" {
                [720., 978.]
            } else {
                [1280., 798.]
            })
            .with_min_inner_size([520., 480.]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "XDJ-1000MK2 Emulator",
        native,
        Box::new(move |cc| Ok(Box::new(DeckApp::new(cc, options)?))),
    )?;
    Ok(())
}
