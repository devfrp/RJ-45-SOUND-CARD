use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[cfg(feature = "gui")]
use rj45_sound_card::gui;
use rj45_sound_card::{audio, client, config, server};

#[derive(Parser)]
#[command(name = "rjsc")]
#[command(about = "RJ45 Sound Card - Share any audio device over Ethernet")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run in SERVER mode: share this PC's audio devices over the network
    Serve {
        /// Path to configuration file
        #[arg(short, long, default_value = "rjsc.toml")]
        config: String,

        /// Run as a daemon (no terminal)
        #[arg(long)]
        daemon: bool,
    },

    /// Run in CLIENT mode: use audio devices from a remote server
    Connect {
        /// Path to configuration file
        #[arg(short, long, default_value = "rjsc.toml")]
        config: String,

        /// Server address (ip:port). Auto-discovers if not specified.
        #[arg(short, long)]
        server: Option<String>,

        /// Run as a daemon (no terminal)
        #[arg(long)]
        daemon: bool,
    },

    /// List all available audio devices on this machine
    List,

    /// Open the graphical control panel (requires 'gui' feature)
    #[cfg(feature = "gui")]
    Gui {
        /// Path to configuration file
        #[arg(short, long, default_value = "rjsc.toml")]
        config: String,

        /// Server address to connect to (optional)
        #[arg(short, long)]
        server: Option<String>,
    },

    /// Generate a default configuration file
    Init {
        /// Path for the new config file
        #[arg(default_value = "rjsc.toml")]
        path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { config, daemon } => {
            let settings = config::load(&config)?;
            let stop_flag = Arc::new(AtomicBool::new(false));
            let sf = stop_flag.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                sf.store(true, Ordering::SeqCst);
            });
            if daemon {
                log::info!("Starting server in daemon mode");
            }
            if let Err(e) = server::run(settings, stop_flag).await {
                log::error!("Server error: {}", e);
                return Err(e);
            }
        }
        Commands::Connect {
            config,
            server,
            daemon,
        } => {
            let settings = config::load(&config)?;
            let stop_flag = Arc::new(AtomicBool::new(false));
            let sf = stop_flag.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                sf.store(true, Ordering::SeqCst);
            });
            if daemon {
                log::info!("Starting client in daemon mode");
            }
            if let Err(e) = client::run(settings, server, stop_flag).await {
                log::error!("Client error: {}", e);
                return Err(e);
            }
        }
        #[cfg(feature = "gui")]
        Commands::Gui { config, server } => {
            let settings = config::load(&config)?;
            gui::run_gui(settings, server)?;
        }
        Commands::List => {
            audio::list_devices()?;
        }
        Commands::Init { path } => {
            config::save_default(&path)?;
            println!("Default configuration written to '{}'", path);
            println!("Edit it to configure your audio devices, then run:");
            println!("  rjsc serve   (to share this PC's audio)");
            println!("  rjsc connect (to use remote audio)");
        }
    }

    Ok(())
}
