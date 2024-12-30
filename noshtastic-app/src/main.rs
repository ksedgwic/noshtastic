use anyhow::Result;
use chrono::Local;
use clap::Parser;
use env_logger::fmt::Formatter;
use env_logger::Builder;
use log::*;
use std::env;
use std::io::Write;
use tokio::signal;
use tokio::time::{sleep, Duration};

use noshtastic_link::create_link;
use noshtastic_sync::Sync;

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

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();
    let args = Args::parse();
    let (linkref, receiver) = create_link(&args.serial).await?;
    let syncref = Sync::new(linkref, receiver)?;

    // give the config a chance to settle before pinging
    sleep(Duration::from_secs(10)).await;

    Sync::start_pinging(syncref.clone(), Duration::from_secs(30))?;

    // wait for termination signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Received ^C, shutting down.");
        }
    }

    syncref.lock().unwrap().stop_pinging()?;

    Ok(())
}
