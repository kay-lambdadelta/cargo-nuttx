use std::{ffi::OsString, num::NonZero};

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

pub const DEFAULT_NUTTX_VERSION: &str = "13.1.0";

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
pub enum Cargo {
    #[command(name = "nuttx", version, about, long_about = None)]
    Nuttx(Cli),
}

#[derive(Args)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn parse() -> Self {
        let mut args: Vec<_> = std::env::args_os().collect();
        let driver_args = args
            .iter()
            .position(|arg| arg == "--")
            .map(|separator| args.split_off(separator))
            .unwrap_or_default();

        let Cargo::Nuttx(mut cli) = Cargo::parse_from(args);
        let common = cli.command.common_mut();
        common.cargo_args.extend(
            driver_args
                .into_iter()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );

        cli
    }
}

impl Command {
    fn common_mut(&mut self) -> &mut CommonArgs {
        match self {
            Command::Build(args) | Command::Run(args) => &mut args.common,
            Command::Check(args) | Command::Clippy(args) => args,
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Build the crate and link it into a NuttX firmware image
    Build(BuildArgs),
    /// Build the firmware image, then hand it to the board's `runner` command
    Run(BuildArgs),
    /// Check the crate against the configured NuttX headers
    Check(CommonArgs),
    /// Run clippy on the crate against the configured NuttX headers
    Clippy(CommonArgs),
}

#[derive(Args)]
pub struct BuildArgs {
    /// Extra argument to pass to `make` when building NuttX (can be repeated)
    #[arg(long = "make-arg", value_name = "ARG", allow_hyphen_values = true)]
    pub make_args: Vec<OsString>,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Args)]
pub struct CommonArgs {
    /// NuttX board to build for, as named by `tools/configure.sh`
    ///
    /// Defaults to the only board in the package metadata, if there is exactly one.
    #[arg(long, short)]
    pub board: Option<String>,
    /// Board configuration to start from
    ///
    /// Defaults to the board's `board-config` metadata, or `nsh`.
    #[arg(long)]
    pub board_config: Option<String>,
    #[command(flatten)]
    pub nuttx: NuttxArgs,
    /// Package to build
    #[arg(long, short)]
    pub package: Option<String>,
    /// Build with the release profile
    #[arg(long, short)]
    pub release: bool,
    /// Number of parallel jobs for cargo and make (default: number of CPUs)
    #[arg(long, short)]
    pub jobs: Option<NonZero<usize>>,
    /// Run `make distclean` and configure NuttX from scratch
    #[arg(long)]
    pub clean: bool,
    /// Extra arguments passed through to cargo; anything after `--` goes to rustc or clippy
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "CARGO_ARGS"
    )]
    pub cargo_args: Vec<String>,
}

#[derive(Args)]
#[group(multiple = false)]
pub struct NuttxArgs {
    /// Use an existing NuttX workspace containing `nuttx` and `apps` directories
    #[arg(long, value_name = "DIR")]
    pub nuttx_path: Option<Utf8PathBuf>,
    /// NuttX release to download into the cache and build against
    ///
    /// Defaults to the `nuttx-version` metadata, or the latest version this tool was tested with.
    #[arg(long, value_name = "VERSION")]
    pub nuttx_version: Option<String>,
}

pub enum NuttxLocation {
    Path { root: Utf8PathBuf },
    Download { version: String },
}

impl NuttxArgs {
    pub fn location(&self, metadata_version: Option<&str>) -> NuttxLocation {
        match (&self.nuttx_path, &self.nuttx_version) {
            (Some(root), _) => NuttxLocation::Path { root: root.clone() },
            (None, version) => NuttxLocation::Download {
                version: version
                    .as_deref()
                    .or(metadata_version)
                    .unwrap_or(DEFAULT_NUTTX_VERSION)
                    .to_string(),
            },
        }
    }
}
