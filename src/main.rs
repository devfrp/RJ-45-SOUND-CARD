use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[cfg(feature = "gui")]
use rj45_sound_card::gui;
use rj45_sound_card::{app, audio, autostart, client, config, logging, server, web};

#[derive(Parser)]
#[command(name = "rjsc")]
#[command(about = "RJ45 Sound Card - Share any audio device over Ethernet")]
#[command(version)]
struct Cli {
    /// Without a command (e.g. double-clicked): same as `start`, and opens the page in the browser.
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the role chosen for this PC (server or laptop), or the setup page on first run
    Start {
        /// Path to configuration file (default: per-user config directory)
        #[arg(short, long)]
        config: Option<String>,

        /// Run in the background (closes the console window on Windows)
        #[arg(long)]
        daemon: bool,
    },

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

    /// Start the server automatically with this computer
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },

    /// Virtual sound card used by the client
    Driver {
        #[command(subcommand)]
        action: DriverAction,
    },
}

#[derive(Subcommand)]
enum DriverAction {
    /// Install it (asks for administrator rights)
    Install,
    /// Show whether it is installed
    Status,
}

#[derive(Subcommand)]
enum AutostartAction {
    /// Run `rjsc start` in the background now and at every startup
    Enable {
        /// Path to configuration file (default: per-user config directory)
        #[arg(short, long)]
        config: Option<String>,

        /// What this PC is (default: the saved role, or server)
        #[arg(long, value_parser = ["server", "client"])]
        role: Option<String>,
    },
    /// Stop and remove the background server
    Disable,
    /// Show whether autostart is enabled
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // `serve` sets up its own logger, which also feeds the web panel.
    // `serve` and `start` set up their own logger, which also feeds the web page.
    if !matches!(
        cli.command,
        None | Some(Commands::Serve { .. } | Commands::Start { .. })
    ) {
        logging::init();
    }

    let Some(command) = cli.command else {
        // Double-clicked: keep running without a console window on Windows.
        #[cfg(windows)]
        if autostart::owns_console() {
            autostart::detach_console();
        }
        return app::run(None, true, logging::init_with_buffer()).await;
    };

    match command {
        Commands::Start { config, daemon } => {
            #[cfg(windows)]
            if daemon {
                autostart::detach_console();
            }
            #[cfg(not(windows))]
            let _ = daemon;
            app::run(config, false, logging::init_with_buffer()).await?;
        }
        Commands::Serve { config, daemon } => {
            #[cfg(windows)]
            if daemon {
                autostart::detach_console();
            }
            let logs = logging::init_with_buffer();
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
            let result = if settings.web.enabled {
                web::serve(settings, config, stop_flag, logs).await
            } else {
                server::run(settings, stop_flag).await
            };
            if let Err(e) = result {
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
        Commands::Autostart { action } => match action {
            AutostartAction::Enable { config, role } => {
                let role = role.map(|r| match r.as_str() {
                    "client" => config::Role::Client,
                    _ => config::Role::Server,
                });
                autostart::enable(config.as_deref(), role)?
            }
            AutostartAction::Disable => autostart::disable()?,
            AutostartAction::Status => autostart::status()?,
        },
        Commands::Driver { action } => match action {
            DriverAction::Install => {
                rj45_sound_card::driver::install()?;
                match rj45_sound_card::driver::detect() {
                    Some(name) => println!("Virtual sound card installed: {}", name),
                    None => println!(
                        "Installer finished, but no virtual sound card is visible yet (a reboot may be needed)."
                    ),
                }
            }
            DriverAction::Status => match rj45_sound_card::driver::detect() {
                Some(name) => println!("Virtual sound card: {}", name),
                None => println!("No virtual sound card installed."),
            },
        },
    }

    Ok(())
}
