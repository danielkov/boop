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
        /// Directory to initialize (relative to cwd)
        dir: Option<String>,
        /// Create a workspace group instead of a leaf
        #[arg(short = 'w', long = "workspace")]
        workspace: bool,
        /// Custom name for the workspace or group
        #[arg(short = 'n', long = "name")]
        name: Option<String>,
        /// Initial version (overrides auto-detection, leaves only)
        #[arg(long)]
        version: Option<String>,
        /// Set as default workspace
        #[arg(long)]
        default: bool,
    },

    /// Create a major change entry
    Major {
        /// Changelog message (markdown)
        message: String,
        /// Target workspace(s), comma-separated
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Target all workspaces (or all within -w groups)
        #[arg(long)]
        all: bool,
    },

    /// Create a minor change entry
    Minor {
        /// Changelog message (markdown)
        message: String,
        /// Target workspace(s), comma-separated
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Target all workspaces (or all within -w groups)
        #[arg(long)]
        all: bool,
    },

    /// Create a patch change entry
    Patch {
        /// Changelog message (markdown)
        message: String,
        /// Target workspace(s), comma-separated
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Target all workspaces (or all within -w groups)
        #[arg(long)]
        all: bool,
    },

    /// Resolve next version and record release
    Apply {
        /// Pre-release tag (use without value for "pre", or specify e.g. "beta")
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        pre: Option<String>,
        /// Merge pending changelog entries into the current version (history rewrite)
        #[arg(long)]
        current: bool,
        /// Target workspace(s), comma-separated
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Apply all workspaces with pending entries
        #[arg(long)]
        all: bool,
        /// Print planned changes without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Query changelog history
    Changelog {
        /// Version or range (e.g. "1.2.3" or "1.0.0...2.0.0")
        range: Option<String>,
        /// Workspace name(s), comma-separated
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Filter by release group ID
        #[arg(long)]
        group: Option<String>,
        /// Print all workspaces from last release group as JSON
        #[arg(long)]
        all_json: bool,
    },

    /// Revert the most recent apply
    Revert,

    /// Show current version and pending entries
    Status {
        /// Scope to a single workspace
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
    },

    /// Print the current version
    Version {
        /// Workspace name
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,
        /// Print all workspaces from last release group as JSON
        #[arg(long)]
        all_json: bool,
    },
}

#[allow(clippy::result_large_err)]
fn run(command: Commands) -> Result<(), boop::errors::BoopError> {
    let base = std::env::current_dir().map_err(|e| boop::errors::StoreError::Read {
        path: std::path::PathBuf::from("."),
        source: e,
    })?;

    match command {
        Commands::Init {
            dir,
            workspace,
            name,
            version,
            default,
        } => {
            boop::commands::init::run(
                &base,
                dir.as_deref(),
                workspace,
                name.as_deref(),
                version.as_deref(),
                default,
            )?;
        }
        Commands::Major {
            message,
            workspace,
            all,
        } => {
            boop::commands::add::run(
                &base,
                boop::version::BumpKind::Major,
                &message,
                workspace.as_deref(),
                all,
            )?;
        }
        Commands::Minor {
            message,
            workspace,
            all,
        } => {
            boop::commands::add::run(
                &base,
                boop::version::BumpKind::Minor,
                &message,
                workspace.as_deref(),
                all,
            )?;
        }
        Commands::Patch {
            message,
            workspace,
            all,
        } => {
            boop::commands::add::run(
                &base,
                boop::version::BumpKind::Patch,
                &message,
                workspace.as_deref(),
                all,
            )?;
        }
        Commands::Apply {
            pre,
            current,
            workspace,
            all,
            dry_run,
        } => {
            let pre_tag = pre
                .as_ref()
                .map(|s| if s.is_empty() { None } else { Some(s.as_str()) });
            boop::commands::apply::run(
                &base,
                pre_tag,
                current,
                workspace.as_deref(),
                all,
                dry_run,
            )?;
        }
        Commands::Changelog {
            range,
            workspace,
            group,
            all_json,
        } => {
            if all_json {
                boop::commands::changelog::run_all_json(&base)?;
            } else {
                boop::commands::changelog::run(
                    &base,
                    range.as_deref(),
                    workspace.as_deref(),
                    group.as_deref(),
                )?;
            }
        }
        Commands::Revert => {
            boop::commands::revert::run(&base)?;
        }
        Commands::Status { workspace } => {
            boop::commands::status::run(&base, workspace.as_deref())?;
        }
        Commands::Version {
            workspace,
            all_json,
        } => {
            if all_json {
                boop::commands::version::run_all_json(&base)?;
            } else {
                boop::commands::version::run(&base, workspace.as_deref())?;
            }
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
