// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use anyhow::Result;
use chrono::Local;
use clap::{CommandFactory, FromArgMatches, Parser};
use directories_next::ProjectDirs;
use env_logger::fmt::Formatter;
use env_logger::Builder;
use log::*;
use nostrdb::{Config, Ndb};
use std::{env, io::Write, path::Path, sync::Arc};
use tokio::{
    signal,
    sync::Notify,
    time::{sleep, Duration},
};

use noshtastic_link::create_link;
use noshtastic_sync::Sync;
use noshtastic_testgw::TestGW;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(
        short,
        long,
        alias = "port",
        help = "the serial device, defaults to auto detect"
    )]
    serial: Option<String>,

    #[arg(
        short,
        long,
        alias = "data-dir",
        help = "The data directory",
        default_value_t = default_data_dir() // Use a function for default value
    )]
    data_dir: String,

    #[arg(
        short = 'r',
        long = "testgw-relay",
        help = "The nostr relay address to use for ingesting test notes, optional"
    )]
    testgw_relay: Option<String>,

    #[arg(
        short = 'f',
        long = "testgw-filter",
        help = "The nostr filter to use for ingesting test notes, optional"
    )]
    testgw_filter: Option<String>,

    #[arg(long = "enable-ping", help = "Enable periodic pings")]
    enable_ping: bool,
}

fn default_data_dir() -> String {
    ProjectDirs::from("com", "bonsai", "noshtastic")
        .map(|dirs| dirs.data_dir().to_string_lossy().to_string())
        .unwrap_or_else(|| "~/.noshtastic".to_string())
}

fn build_args_with_help() -> Result<Args> {
    let default_dir = default_data_dir();
    let default_dir_static: &'static str = Box::leak(default_dir.into_boxed_str());
    let mut cmd = Args::command();
    cmd = cmd.mut_arg("data_dir", |arg| {
        arg.help(format!(
            "The data directory, defaults to {}",
            default_dir_static
        ))
        .default_value(default_dir_static) // Set the default value dynamically
    });
    Ok(Args::from_arg_matches(&cmd.get_matches())?)
}

fn init_logger() {
    let mut builder = Builder::new();
    if let Ok(rust_log) = env::var("RUST_LOG") {
        builder.parse_filters(&rust_log);
    } else {
        builder.filter(None, log::LevelFilter::Debug); // Default to Debug
    }
    builder
        .format(|buf: &mut Formatter, record: &Record| {
            let now = Local::now();
            writeln!(
                buf,
                "{} {:<5} {}: {}",
                now.format("%H:%M:%S%.3f"),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .init();
}

fn init_nostrdb(data_dir: &str) -> Result<Ndb> {
    let datapath = Path::new(data_dir);
    let dbpath = datapath.join("db");
    let mapsize = if cfg!(target_os = "windows") {
        // 16 Gib on windows because it actually creates the file
        1024usize * 1024usize * 1024usize * 16usize
    } else {
        // 1 TiB for everything else since its just virtually mapped
        1024usize * 1024usize * 1024usize * 1024usize
    };
    let config = Config::new().set_ingester_threads(4).set_mapsize(mapsize);
    Ok(Ndb::new(&dbpath.to_string_lossy(), &config)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();
    let stop_signal = Arc::new(Notify::new());
    let args = build_args_with_help()?;
    let ndb = init_nostrdb(&args.data_dir)?;
    let mut testgw = TestGW::new(ndb.clone(), &args.testgw_relay, &args.testgw_filter)?;
    let (_linkref, link_tx, link_rx) = create_link(&args.serial, stop_signal.clone()).await?;
    let syncref = Sync::new(ndb.clone(), link_tx, link_rx, stop_signal.clone())?;

    testgw.start()?;
    if args.enable_ping {
        // give the config a chance to settle before pinging
        sleep(Duration::from_secs(5)).await;
        Sync::start_pinging(syncref.clone(), Duration::from_secs(30))?;
    }

    // wait for termination signal
    info!("waiting for ^C to terminate ...");
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Received ^C, shutting down.");
            stop_signal.notify_waiters();
        }
    }

    if args.enable_ping {
        syncref.lock().unwrap().stop_pinging()?;
    }
    testgw.stop()?;

    Ok(())
}
