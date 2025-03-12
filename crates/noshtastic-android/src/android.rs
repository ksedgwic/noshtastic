use chrono::Local;
use eframe::egui;
use egui_winit::winit::platform::android::activity::AndroidApp;
use env_logger;
use log::*;
use once_cell::sync::Lazy;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// RUST_LOG style log spec
const LOG_SPEC: &str = concat!(
    // Default logging level
    "info",
    // Quieting down noisy dependencies
    ",wgpu_hal=error",
    ",wgpu_core=error",
    ",egui_wgpu=warn",
    // Debugging these specific modules
    ",noshtastic_=debug",
);

const MAX_MESSAGES: usize = 10_000;
const PERIODIC_DURATION: Duration = Duration::from_secs(10);

/// Global log buffer that stores formatted log messages.
static LOG_MESSAGES: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

/// A custom writer that appends log output to the global buffer.
struct GuiWriter {
    messages: Arc<Mutex<Vec<String>>>,
}

impl Write for GuiWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Convert the byte slice to a string.
        let s = std::str::from_utf8(buf).unwrap_or("<invalid utf8>");
        let mut msgs = self.messages.lock().unwrap();
        // Split the string into lines and store each line.
        for line in s.lines() {
            msgs.push(line.to_string());
        }
        // If we exceed MAX_MESSAGES, drop the oldest ones.
        if msgs.len() > MAX_MESSAGES {
            let excess = msgs.len() - MAX_MESSAGES;
            msgs.drain(0..excess);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn init_logger() {
    let gui_writer = GuiWriter {
        messages: LOG_MESSAGES.clone(),
    };

    let mut builder = env_logger::Builder::new();
    builder.parse_filters(LOG_SPEC);

    builder.format(|buf, record| {
        let now = Local::now();
        writeln!(
            buf,
            "{} {:<5} {}: {}",
            now.format("%H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        )
    });

    // Redirect logger output to our GuiWriter.
    builder.target(env_logger::Target::Pipe(Box::new(gui_writer)));
    builder.init();
}

/// egui application that displays log messages from the global buffer.
struct LogApp {
    messages: Arc<Mutex<Vec<String>>>,
    last_periodic_tick: Instant,
}

impl LogApp {
    fn new() -> Self {
        Self {
            messages: LOG_MESSAGES.clone(),
            last_periodic_tick: Instant::now(),
        }
    }
}

impl eframe::App for LogApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request continuous repaint (approximately 60 FPS).
        ctx.request_repaint_after(Duration::from_millis(16));

        if self.last_periodic_tick.elapsed() >= PERIODIC_DURATION {
            self.last_periodic_tick = Instant::now();
            debug!("Periodic");
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Log Output");
            ui.separator();
            // Auto-scroll if at bottom.
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let msgs = self.messages.lock().unwrap();
                    for msg in msgs.iter() {
                        ui.label(msg);
                    }
                });
        });
    }
}

#[no_mangle]
#[tokio::main]
pub async fn android_main(app: AndroidApp) {
    init_logger();

    log::info!("noshtastic starting");

    let mut options = eframe::NativeOptions::default();
    options.renderer = eframe::Renderer::Wgpu;
    options.android_app = Some(app.clone());

    let _res = eframe::run_native(
        "Noshtastic Android Logger",
        options,
        Box::new(|_cc| Ok(Box::new(LogApp::new()))),
    );
}
