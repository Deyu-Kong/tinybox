mod cgroup;
mod rootfs;
mod sandbox;

use anyhow::Result;
use cgroup::parse_mem_limit;
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

        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        cpu_limit: Option<u32>,

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
            root,
            hostname,
            mem_limit,
            cpu_limit,
            dangerous: _,
            proxy: _,
            oci: _,
        } => {
            let mem_limit_bytes = match mem_limit {
                Some(s) => Some(parse_mem_limit(&s)?),
                None => None,
            };
            let config = SandboxConfig {
                command,
                hostname,
                rootfs: root.map(std::path::PathBuf::from),
                mem_limit: mem_limit_bytes,
                cpu_limit,
            };
            let code = run_sandbox(&config)?;
            std::process::exit(code);
        }
    }
}
