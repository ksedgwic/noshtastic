use egui_winit::winit::platform::android::activity::AndroidApp;

pub struct NoshApp {
    // stuff here
}

impl eframe::App for NoshApp {
    fn update(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {}
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {}
}

impl NoshApp {
    pub fn new(_ctx: &egui::Context) -> Self {
        Self {}
    }
}

#[no_mangle]
#[tokio::main]
pub async fn android_main(app: AndroidApp) {
    use tracing_logcat::{LogcatMakeWriter, LogcatTag};
    use tracing_subscriber::{prelude::*, EnvFilter};

    std::env::set_var("RUST_BACKTRACE", "full");
    std::env::set_var("RUST_LOG", "egui=trace,android_activity=debug");

    let _writer =
        LogcatMakeWriter::new(LogcatTag::Target).expect("Failed to initialize logcat writer");

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_level(false)
        .with_target(false)
        .without_time();

    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    // let path = app.internal_data_path().expect("data path");
    let mut options = eframe::NativeOptions::default();
    options.renderer = eframe::Renderer::Wgpu;
    options.android_app = Some(app.clone());

    let _res = eframe::run_native(
        "Noshtastic",
        options,
        Box::new(move |cc| {
            let ctx = &cc.egui_ctx;
            let noshapp = NoshApp::new(ctx);
            Ok(Box::new(noshapp))
        }),
    );
}
