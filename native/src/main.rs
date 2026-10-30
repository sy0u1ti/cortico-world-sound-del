mod controller;
mod hotkey;
mod journal;
mod key_input;
mod latency;
mod measured;
mod notifications;
mod runtime;
mod simulation;
mod supervisor;
#[cfg(windows)]
mod windows_audio;
mod worker;

use anyhow::{Context, Result, bail};
use serde_json::json;
use std::path::PathBuf;

struct Options {
    directory: Option<PathBuf>,
    simulation_file: Option<PathBuf>,
    probe: bool,
    recover_only: bool,
    worker: bool,
    maximum_seconds: u64,
}

fn options() -> Result<Options> {
    let mut options = Options {
        directory: None,
        simulation_file: None,
        probe: false,
        recover_only: false,
        worker: false,
        maximum_seconds: 300,
    };
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                options.directory = Some(args.next().context("--data-dir requires a path")?.into())
            }
            Some("--simulation-file") => {
                options.simulation_file = Some(
                    args.next()
                        .context("--simulation-file requires a path")?
                        .into(),
                )
            }
            Some("--probe") => options.probe = true,
            Some("--recover-only") => options.recover_only = true,
            Some("--worker") => options.worker = true,
            Some("--max-mute-seconds") => {
                options.maximum_seconds = args
                    .next()
                    .context("--max-mute-seconds requires a number")?
                    .to_str()
                    .context("Invalid maximum duration")?
                    .parse()?;
            }
            _ => bail!("Unknown argument"),
        }
    }
    if options.probe && (options.recover_only || options.worker) {
        bail!("Probe cannot be combined with recovery or worker mode");
    }
    if !(10..=900).contains(&options.maximum_seconds) {
        bail!("Maximum duration must be 10..900 seconds");
    }
    if !options.probe && options.directory.is_none() {
        bail!("--data-dir is required");
    }
    Ok(options)
}

fn run() -> Result<()> {
    let options = options()?;
    if options.worker || options.probe {
        worker::run(options)
    } else {
        supervisor::run(options)
    }
}

fn main() {
    if let Err(error) = run() {
        runtime::Reporter::new()
            .finish(json!({ "type": "error", "message": format!("{error:#}") }));
        std::process::exit(1);
    }
}
