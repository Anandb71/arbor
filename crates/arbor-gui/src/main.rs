//! Arbor GUI - Impact-first code analysis interface
//!
//! A minimal, focused GUI for answering: "What breaks if I change this?"

mod app;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let icon = load_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Arbor - Impact Analysis")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "Arbor",
        options,
        Box::new(|cc| {
            // Install image loaders for SVG support
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::ArborApp::new(cc)))
        }),
    )
}

fn load_icon() -> std::sync::Arc<egui::IconData> {
    // Embed the SVG logo
    let svg_data = include_bytes!("../../../docs/assets/arbor-logo.svg");

    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_data, &options).expect("Failed to parse SVG icon");

    let size = tree.size();
    let width = size.width().ceil() as u32;
    let height = size.height().ceil() as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).expect("Failed to create pixmap");
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());

    std::sync::Arc::new(egui::IconData {
        rgba: pixmap.take(),
        width,
        height,
    })
}
