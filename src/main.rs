use anyhow::Result;
use clap::{Parser, Subcommand};
use cartridge::{GameBackup, find_config_file};

#[derive(Parser)]
#[command(name = "cartridge")]
#[command(about = "A CLI tool for backing up and restoring game save files")]
#[command(version = "0.1.0")]
struct Cli {
    /// Path to the TOML configuration file
    #[arg(short, long)]
    config: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Backup game saves
    Backup {
        /// Name of the game to backup (if not specified, backup all games)
        game_name: Option<String>,
    },
    /// Restore game saves
    Restore {
        /// Name of the game to restore (if not specified, restore all games)
        game_name: Option<String>,
    },
    /// List all games in configuration
    List,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logger
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp_secs()
        .init();

    log::info!("Starting Game Backup CLI v{}", env!("CARGO_PKG_VERSION"));

    // Find and load configuration
    let config_path = find_config_file(cli.config.as_deref())?;
    let game_backup = GameBackup::new(&config_path)?;

    // Execute command
    match cli.command {
        Commands::Backup { game_name } => {
            if let Some(name) = game_name {
                game_backup.backup_game(&name)?;
            } else {
                game_backup.backup_all_games()?;
            }
        }
        Commands::Restore { game_name } => {
            if let Some(name) = game_name {
                game_backup.restore_game(&name)?;
            } else {
                game_backup.restore_all_games()?;
            }
        }
        Commands::List => {
            let games = game_backup.list_games();
            if games.is_empty() {
                println!("No enabled games found in configuration.");
            } else {
                println!("Available games:");
                for game in games {
                    let has_backup = game_backup.has_backup(&game.name);
                    let backup_status = if has_backup {
                        "Has backup"
                    } else {
                        "No backup"
                    };
                    println!(
                        "  {} - {} ({} save locations)",
                        game.name,
                        backup_status,
                        game.saves.len()
                    );
                }
            }
        }
    }
    Ok(())
}
