mod audit;
mod broker;
mod cgroup;
mod daemon;
mod exec;
mod image;
mod landlock;
mod oci;
mod policy;
mod proxy;
mod registry;
mod rootfs;
mod sandbox;
mod seccomp;
mod task;

use anyhow::{Context, Result};
use cgroup::parse_memory;
use clap::{Parser, Subcommand};
use sandbox::{run_sandbox, SandboxConfig};

#[derive(Parser)]
#[command(
    name = "tinybox",
    version,
    about = "A minimal Linux sandbox runtime",
    long_about = "A minimal Linux sandbox runtime.\n\nWARNING: tinybox is experimental, rootful software and is not a production security boundary."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
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

        /// Apply a versioned Agent capability policy (experimental).
        #[arg(long, value_name = "PATH")]
        policy: Option<String>,

        #[arg(long)]
        oci: Option<String>,

        #[arg(long)]
        image: Option<String>,

        #[arg(long = "read-only")]
        read_only: bool,

        #[arg(short = 'v', long = "volume")]
        volumes: Vec<String>,

        #[arg(last = true)]
        command: Vec<String>,
    },
    Exec {
        #[arg(long)]
        pid: u32,

        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Launch a host Agent under the policy's immutable Landlock FS ceiling.
    AgentHost {
        #[arg(long, value_name = "PATH")]
        policy: String,

        #[arg(last = true, required = true)]
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
    Pull {
        image: String,
        #[arg(long)]
        alias: Option<String>,
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
            ImageAction::Pull { image, alias } => {
                let image_ref = registry::ImageRef::parse(&image)?;
                let alias = alias.unwrap_or_else(|| {
                    image_ref
                        .repository
                        .split('/')
                        .next_back()
                        .unwrap_or("pulled")
                        .to_string()
                });
                let dest = image::image_store().join(&alias);
                if dest.exists() {
                    anyhow::bail!("image alias already exists: {}", dest.display());
                }
                registry::pull(&image_ref, &dest)?;
                println!("pulled: {} -> {}", image, dest.display());
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
            policy,
            oci,
            image,
            read_only,
            volumes,
        } => {
            let (command, root, oci_env, root_readonly, cwd, uid, gid, namespaces) =
                if let Some(bundle_path) = oci {
                    let bundle = oci::load_bundle(std::path::Path::new(&bundle_path))?;
                    (
                        bundle.command,
                        Some(bundle.rootfs.to_string_lossy().into_owned()),
                        bundle.env,
                        bundle.root_readonly,
                        bundle.cwd,
                        bundle.uid,
                        bundle.gid,
                        bundle.namespaces,
                    )
                } else if let Some(image_name) = image {
                    let path = image::resolve(&image_name)?;
                    (
                        command,
                        Some(path.to_string_lossy().into_owned()),
                        Vec::new(),
                        read_only,
                        None,
                        0,
                        0,
                        None,
                    )
                } else {
                    (command, root, Vec::new(), read_only, None, 0, 0, None)
                };
            let loaded_policy = match policy {
                Some(path) => {
                    if dangerous {
                        anyhow::bail!("--dangerous cannot be combined with --policy");
                    }
                    Some(policy::CapabilityDescriptor::load(std::path::Path::new(
                        &path,
                    ))?)
                }
                None => {
                    eprintln!(
                        "WARNING: running without --policy uses legacy sandbox configuration"
                    );
                    None
                }
            };
            if loaded_policy
                .as_ref()
                .is_some_and(|policy| !policy.descriptor.phases.is_empty())
            {
                anyhow::bail!("phase policies require daemon mode and its control API");
            }
            let memory_bytes = match memory {
                Some(s) => Some(parse_memory(&s)?),
                None => None,
            };
            let (memory_bytes, cpus, pids_limit) = if let Some(policy) = &loaded_policy {
                let resources = &policy.descriptor.resources;
                if memory_bytes.is_some_and(|value| value > resources.memory_bytes) {
                    anyhow::bail!("--memory exceeds the policy ceiling");
                }
                if cpus.is_some_and(|value| value > resources.cpus) {
                    anyhow::bail!("--cpus exceeds the policy ceiling");
                }
                if pids_limit.is_some_and(|value| value > resources.pids) {
                    anyhow::bail!("--pids-limit exceeds the policy ceiling");
                }
                (
                    memory_bytes.or(Some(resources.memory_bytes)),
                    cpus.or(Some(resources.cpus)),
                    pids_limit.or(Some(resources.pids)),
                )
            } else {
                (memory_bytes, cpus, pids_limit)
            };
            if let Some(policy) = &loaded_policy {
                eprintln!("tinybox policy: {}", policy.hash);
            }
            let config = SandboxConfig {
                cgroup_name: None,
                command,
                hostname,
                rootfs: root.map(std::path::PathBuf::from),
                root_readonly,
                env: oci_env,
                proxy,
                volumes,
                memory: memory_bytes,
                cpus,
                cpu_quota,
                cpu_period,
                pids_limit,
                dangerous,
                filesystem_policy: loaded_policy
                    .as_ref()
                    .map(|policy| policy.descriptor.filesystem.clone()),
                network_policy: loaded_policy.as_ref().map(|policy| {
                    std::sync::Arc::new(std::sync::RwLock::new(policy.descriptor.network.clone()))
                }),
                audit: None,
                namespaces,
                cwd,
                uid,
                gid,
            };
            let code = run_sandbox(&config)?;
            std::process::exit(code);
        }
        Commands::Exec { pid, command } => {
            let code = exec::exec_in_container(pid, &command)?;
            std::process::exit(code);
        }
        Commands::AgentHost { policy, command } => {
            let loaded = policy::CapabilityDescriptor::load(std::path::Path::new(&policy))?;
            if !loaded.descriptor.phases.is_empty() {
                anyhow::bail!("host Agent launcher does not support dynamic phase policies");
            }
            landlock::enforce(&loaded.descriptor.filesystem)?;
            let program = std::ffi::CString::new(command[0].as_str())?;
            let args: Vec<std::ffi::CString> = command
                .iter()
                .map(|value| std::ffi::CString::new(value.as_str()))
                .collect::<std::result::Result<_, _>>()?;
            nix::unistd::execvp(&program, &args).context("failed to exec host Agent command")?;
        }
    }
    Ok(())
}
