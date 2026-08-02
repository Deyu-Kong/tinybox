mod cgroup;
mod rootfs;
mod sandbox;

use anyhow::Result;
use cgroup::parse_memory;
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

        #[arg(short = 'm', long)]
        memory: Option<String>,

        #[arg(long)]
        cpus: Option<f64>,

        #[arg(long)]
        pids_limit: Option<u64>,

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
            memory,
            cpus,
            pids_limit,
            dangerous: _,
            proxy: _,
            oci: _,
        } => {
            let memory_bytes = match memory {
                Some(s) => Some(parse_memory(&s)?),
                None => None,
            };
            let config = SandboxConfig {
                command,
                hostname,
                rootfs: root.map(std::path::PathBuf::from),
                memory: memory_bytes,
                cpus,
                pids_limit,
            };
            let code = run_sandbox(&config)?;
            std::process::exit(code);
        }
    }
}
