use anyhow::Result;
use clap::Parser;
use log::*;
use tokio::signal;
use tokio::time::Duration;

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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let (linkref, receiver) = create_link(&args.serial).await?;
    let syncref = Sync::new(linkref, receiver)?;

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
