//! Focale desktop application (winit + wgpu + egui via eframe).

mod app;
mod export_queue;
mod jobs;
mod panels;
mod preview;
mod session;
mod suggest;
mod thumbs;
mod viewport;

fn main() -> eframe::Result {
    tracing_subscriber();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(format!(
                "Focale — pipeline v{}",
                focale_core::PIPELINE_VERSION
            ))
            .with_inner_size([1600.0, 1000.0]),
        ..Default::default()
    };
    eframe::run_native(
        "focale",
        options,
        Box::new(|cc| Ok(Box::new(app::FocaleApp::new(cc)))),
    )
}

/// Minimal env-filtered logging to stderr.
fn tracing_subscriber() {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
