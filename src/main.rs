use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "boop", about = "Language-agnostic release management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .boop/ directory
    Init {
        /// Initial version (overrides auto-detection)
        #[arg(long)]
        version: Option<String>,
    },

    /// Create a major change entry
    Major {
        /// Changelog message (markdown)
        message: String,
    },

    /// Create a minor change entry
    Minor {
        /// Changelog message (markdown)
        message: String,
    },

    /// Create a patch change entry
    Patch {
        /// Changelog message (markdown)
        message: String,
    },

    /// Resolve next version and record release
    Apply {
        /// Pre-release tag (use without value for "pre", or specify e.g. "beta")
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        pre: Option<String>,
    },

    /// Query changelog history
    Changelog {
        /// Version or range (e.g. "1.2.3" or "1.0.0...2.0.0")
        range: Option<String>,
    },

    /// Show current version and pending entries
    Status,

    /// Print the current version
    Version,
}

#[allow(clippy::result_large_err)]
fn run(command: Commands) -> Result<(), boop::errors::BoopError> {
    let base = std::env::current_dir().map_err(|e| boop::errors::StoreError::Read {
        path: std::path::PathBuf::from("."),
        source: e,
    })?;

    match command {
        Commands::Init { version } => {
            boop::commands::init::run(&base, version.as_deref())?;
        }
        Commands::Major { message } => {
            boop::commands::add::run(&base, boop::version::BumpKind::Major, &message)?;
        }
        Commands::Minor { message } => {
            boop::commands::add::run(&base, boop::version::BumpKind::Minor, &message)?;
        }
        Commands::Patch { message } => {
            boop::commands::add::run(&base, boop::version::BumpKind::Patch, &message)?;
        }
        Commands::Apply { pre } => {
            let pre_tag = pre
                .as_ref()
                .map(|s| if s.is_empty() { None } else { Some(s.as_str()) });
            boop::commands::apply::run(&base, pre_tag)?;
        }
        Commands::Changelog { range } => {
            boop::commands::changelog::run(&base, range.as_deref())?;
        }
        Commands::Status => {
            boop::commands::status::run(&base)?;
        }
        Commands::Version => {
            boop::commands::version::run(&base)?;
        }
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli.command) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
