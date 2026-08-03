mod cgroup;
mod daemon;
mod image;
mod oci;
mod rootfs;
mod sandbox;
mod seccomp;

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
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
    },
    Image {
        #[command(subcommand)]
        action: ImageAction,
    },
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
        cpu_quota: Option<i64>,

        #[arg(long)]
        cpu_period: Option<u64>,

        #[arg(long)]
        pids_limit: Option<u64>,

        #[arg(long)]
        dangerous: bool,

        #[arg(long)]
        proxy: Option<String>,

        #[arg(long)]
        oci: Option<String>,

        #[arg(long)]
        image: Option<String>,

        #[arg(last = true)]
        command: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ImageAction {
    Import {
        tar: String,
        #[arg(long, default_value = "default")]
        alias: String,
    },
    List,
    Remove {
        alias: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon { listen } => {
            let address = daemon::parse_listen(&listen)?;
            tokio::runtime::Runtime::new()?.block_on(daemon::serve(address))?;
        }
        Commands::Image { action } => match action {
            ImageAction::Import { tar, alias } => {
                let dest = image::import_tar(std::path::Path::new(&tar), &alias)?;
                println!("imported: {}", dest.display());
            }
            ImageAction::List => {
                for name in image::list()? {
                    println!("{name}");
                }
            }
            ImageAction::Remove { alias } => {
                image::remove(&alias)?;
                println!("removed: {alias}");
            }
        },
        Commands::Run {
            command,
            root,
            hostname,
            memory,
            cpus,
            cpu_quota,
            cpu_period,
            pids_limit,
            dangerous,
            proxy,
            oci,
            image,
        } => {
            let (command, root, oci_env) = if let Some(bundle_path) = oci {
                let bundle = oci::load_bundle(std::path::Path::new(&bundle_path))?;
                (
                    bundle.command,
                    Some(bundle.rootfs.to_string_lossy().into_owned()),
                    bundle.env,
                )
            } else if let Some(image_name) = image {
                let path = image::resolve(&image_name)?;
                (
                    command,
                    Some(path.to_string_lossy().into_owned()),
                    Vec::new(),
                )
            } else {
                (command, root, Vec::new())
            };
            let memory_bytes = match memory {
                Some(s) => Some(parse_memory(&s)?),
                None => None,
            };
            let config = SandboxConfig {
                command,
                hostname,
                rootfs: root.map(std::path::PathBuf::from),
                env: oci_env,
                proxy,
                memory: memory_bytes,
                cpus,
                cpu_quota,
                cpu_period,
                pids_limit,
                dangerous,
            };
            let code = run_sandbox(&config)?;
            std::process::exit(code);
        }
    }
    Ok(())
}
