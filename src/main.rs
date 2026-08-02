mod sandbox;

use anyhow::Result;
use clap::{Parser, Subcommand};
use sandbox::{run_sandbox, SandboxConfig};

#[derive(Parser)]
#[command(name = "tinybox", version, about = "A minimal Linux sandbox runtime")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        root: Option<String>,

        #[arg(long)]
        hostname: Option<String>,

        #[arg(long)]
        mem_limit: Option<String>,

        #[arg(long)]
        dangerous: bool,

        #[arg(long)]
        proxy: Option<String>,

        #[arg(long)]
        oci: Option<String>,

        #[arg(last = true)]
        command: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            command,
            root: _,
            hostname,
            mem_limit: _,
            dangerous: _,
            proxy: _,
            oci: _,
        } => {
            let config = SandboxConfig { command, hostname };
            let code = run_sandbox(&config)?;
            std::process::exit(code);
        }
    }
}
