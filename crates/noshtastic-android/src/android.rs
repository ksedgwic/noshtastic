// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use android_logger::Config as AndroidLogConfig;
use android_logger::Filter;
use android_logger::FilterBuilder;
use anyhow::{Context, Result};
use btleplug;
use chrono::Local;
use jni::objects::GlobalRef;
use jni::objects::{JObject, JValue};
use jni::sys::jint;
use jni::JNIEnv;
use jni::JavaVM;
use log::*;
use nostrdb::{Config, Ndb};
use once_cell::sync::{Lazy, OnceCell};
use std::os::raw::c_void;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

use noshtastic_link::create_link;
use noshtastic_link::LinkRef;
use noshtastic_sync::sync::SyncRef;
use noshtastic_sync::Sync;

// BLE device to connect to
// kmem:
static MAYBE_HINT: Lazy<Option<String>> = Lazy::new(|| Some("64:E8:33:47:07:C1".to_string()));
// static MAYBE_HINT: Lazy<Option<String>> = Lazy::new(|| Some("D4:73:31:4E:9F:3B".to_string()));
// eph2: D4:73:31:4E:9F:3B
// kmem: 64:E8:33:47:07:C1

// Our global tokio runtime handle
static GLOBAL_RT: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
// Our global JVM reference, set during JNI_OnLoad
static GLOBAL_JVM: OnceCell<JavaVM> = OnceCell::new();
// Our main activity class
static GLOBAL_MAIN_ACTIVITY: OnceCell<GlobalRef> = OnceCell::new();

// This struct tracks your link + sync references
pub static GLOBAL_STATE: Lazy<Mutex<AppState>> = Lazy::new(|| Mutex::new(AppState::new()));

pub struct AppState {
    pub link: Option<LinkRef>,
    pub sync: Option<SyncRef>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            link: None,
            sync: None,
        }
    }
}

/// Minimal LMDB/nostrdb init
fn init_nostrdb() -> Result<Ndb> {
    let dbpath = "/data/user/0/com.bonsai.noshtastic/files/db".to_string();
    let _ = std::fs::create_dir_all(&dbpath);
    let mapsize = 1024usize * 1024usize * 1024usize * 1024usize;
    let config = Config::new().set_ingester_threads(4).set_mapsize(mapsize);
    Ok(Ndb::new(&dbpath, &config)?)
}

// --------------------------------------------------------
// HELPER FUNCTIONS

pub fn condition_thread_for_jni() {
    // Fail immediately if GLOBAL_JVM isn't set
    let vm = GLOBAL_JVM
        .get()
        .expect("GLOBAL_JVM not set! JNI_OnLoad might be missing?");

    // Attach to this worker thread
    let _env = vm
        .attach_current_thread_permanently()
        .expect("Failed to attach_current_thread");

    debug!("thread attached to JVM");
}

// --------------------------------------------------------
// LOGGING
use android_logger::AndroidLogger;

fn redirect_stdout_stderr_to_logcat() {
    use std::{fs::File, io::Read, os::fd::FromRawFd};

    unsafe {
        let mut pipes: [libc::c_int; 2] = [0, 0];
        // create a pipe: pipes[0] is read end, pipes[1] is write end
        if libc::pipe(pipes.as_mut_ptr()) == 0 {
            let read_fd = pipes[0];
            let write_fd = pipes[1];

            // send both stdout and stderr to the pipe’s write end
            libc::dup2(write_fd, libc::STDOUT_FILENO);
            libc::dup2(write_fd, libc::STDERR_FILENO);

            // close the extra file descriptor
            libc::close(write_fd);

            // read side must remain open, so spawn a thread to forward data
            std::thread::spawn(move || {
                let mut reader = File::from_raw_fd(read_fd);
                let mut buf = [0u8; 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let s = String::from_utf8_lossy(&buf[..n]);
                            // Forward each chunk to the Android logs
                            // e.g. chunk by chunk or line by line
                            log::info!("STDOUT/STDERR: {}", s.trim_end());
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    }
}

// format a log record for adb logcat
fn log_format_adb(buf: &mut dyn std::fmt::Write, record: &log::Record) -> std::fmt::Result {
    // Example format: "[mycrate] (file:line) message"
    write!(
        buf,
        "[{}] ({}:{}) {}",
        record.target(),
        record.file_static().unwrap_or("??"),
        record.line().unwrap_or(0),
        record.args()
    )?;
    buf.write_char('\n')?;
    Ok(())
}

// format a log record for the GUI log view (leave out file and line, don't need \n)
fn log_format_gui(buf: &mut dyn std::fmt::Write, record: &log::Record) -> std::fmt::Result {
    let now_str = Local::now().format("%m-%d %H:%M:%S%.3f");
    // Example format: "04-21 19:49:13.171 INFO [mycrate] message"
    write!(
        buf,
        "{} {:<5} [{}] {}",
        now_str,
        record.level(),
        record.target(),
        record.args()
    )?;
    Ok(())
}

struct DualLogger {
    android_logger: AndroidLogger,
    gui_filter: Filter,
}

impl DualLogger {
    fn new() -> Self {
        // what should we log to adb?
        const ADB_SPEC: &str = concat!(
            // Default logging level
            "debug",
            // Quieting down noisy dependencies
            ",wgpu_hal=error",
            ",wgpu_core=error",
            ",egui_wgpu=warn",
            ",meshtastic::connections::stream_buffer=off",
            ",jni::wrapper::java_vm::vm=info",
            // Debugging these specific modules
        );
        let adb_filter = FilterBuilder::new().parse(ADB_SPEC).build();

        // what should we log to the GUI?
        const GUI_SPEC: &str = concat!(
            // Default logging level
            "debug",
            // Quieting down noisy dependencies
            ",wgpu_hal=error",
            ",wgpu_core=error",
            ",egui_wgpu=warn",
            ",meshtastic::connections::stream_buffer=off",
            ",jni::wrapper::java_vm::vm=info",
            // Debugging these specific modules
        );
        let gui_filter = FilterBuilder::new().parse(GUI_SPEC).build();

        let android_logger = android_logger::AndroidLogger::new(
            AndroidLogConfig::default()
                .with_max_level(LevelFilter::Debug)
                .with_tag("NOSH")
                .with_filter(adb_filter)
                .format(log_format_adb),
        );

        Self {
            android_logger,
            gui_filter,
        }
    }
}

impl log::Log for DualLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            if self.android_logger.enabled(record.metadata()) {
                self.android_logger.log(record);
            }
            if self.gui_filter.enabled(record.metadata()) {
                let mut buf = String::new();
                if log_format_gui(&mut buf, record).is_ok() {
                    send_log_msg_to_gui(&buf);
                }
            }
        }
    }

    fn flush(&self) {}
}

fn send_log_msg_to_gui(msg: &str) {
    let env = GLOBAL_JVM
        .get()
        .expect("GLOBAL_JVM not initialized")
        .attach_current_thread()
        .expect("Failed to attach thread");

    let jmsg = env.new_string(msg).unwrap();

    env.call_method(
        GLOBAL_MAIN_ACTIVITY.get().unwrap().as_obj(),
        "appendLog",
        "(Ljava/lang/String;)V",
        &[JValue::Object(jmsg.into())],
    )
    .expect("Failed calling appendLog");
}

fn initialize_logging(_env: &JNIEnv) {
    let dual_logger = DualLogger::new();

    log::set_boxed_logger(Box::new(dual_logger))
        .expect("global logger already set?!  Cannot install two loggers");
    log::set_max_level(LevelFilter::Debug);

    log::info!("logging initialized");

    redirect_stdout_stderr_to_logcat();
}

// --------------------------------------------------------
// JNI ENTRY POINTS

/// Called by the JVM when our .so is first loaded
#[no_mangle]
pub extern "C" fn JNI_OnLoad(_vm: JavaVM, _: *const c_void) -> jint {
    // enable rust backtraces
    std::env::set_var("RUST_BACKTRACE", "1");

    jni::JNIVersion::V6.into()
}

/// Called from onCreate:
/// - using a UI thread which has a capable classloader
/// - after the logging view is setup (ok to call appendLog
#[no_mangle]
pub extern "system" fn Java_com_bonsai_noshtastic_MainActivity_onCreateNative(
    env: JNIEnv,
    activity: JObject,
) {
    // thread conditioning not required here because this is called on
    // the main java thread which is already connected to the JVM

    let vm = env.get_java_vm().unwrap();
    GLOBAL_JVM.set(vm).ok();

    // Create a tokio runtime *once*, storing it in GLOBAL_RT
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_start(|| {
            condition_thread_for_jni();
        })
        .build()
        .expect("Failed to build tokio runtime");

    GLOBAL_RT.set(runtime).ok();

    let global_activity = env
        .new_global_ref(activity)
        .expect("Failed to create global ref to MainActivity instance");
    GLOBAL_MAIN_ACTIVITY.set(global_activity).ok();

    initialize_logging(&env);

    info!("onCreateNative continuing");

    // Here you can do additional JNI initialization, if needed
    if let Err(e) = jni_utils::init(&env) {
        error!("jni_utils::init failed: {:?}", e);
    } else {
        info!("jni_utils::init succeeded");
    }
    if let Err(e) = btleplug::platform::init(&env) {
        error!("btleplug::platform::init failed: {:?}", e);
    } else {
        info!("btleplug::platform::init succeeded");
    }

    info!("onCreateNative finished");
}

/// Called from Java after BLE permissions are granted
#[no_mangle]
pub extern "system" fn Java_com_bonsai_noshtastic_MainActivity_startNoshtastic(
    _env: JNIEnv,
    _activity: JObject,
) {
    info!("startNoshtastic starting");
    GLOBAL_RT
        .get()
        .expect("GLOBAL_RT was never set!")
        .spawn(async {
            match setup_noshtastic().await {
                Ok(()) => debug!("setup_noshtastic finished"),
                Err(e) => error!("setup_noshtastic failed: {e:?}"),
            }
        });
    info!("startNoshtastic finished");
}

async fn setup_noshtastic() -> Result<()> {
    debug!("setup noshtastic starting");
    let stop_signal = Arc::new(Notify::new());

    let (linkref, link_tx, link_rx) = create_link(&MAYBE_HINT, stop_signal.clone())
        .await
        .context("create_link failed")?;

    let ndb = init_nostrdb().context("init_nostrdb failed")?;
    let syncref = Sync::new(ndb, link_tx, link_rx, stop_signal).context("Sync::new failed")?;

    {
        let mut state = GLOBAL_STATE.lock().unwrap();
        state.link = Some(linkref);
        state.sync = Some(syncref);
    }
    Ok(())
}
