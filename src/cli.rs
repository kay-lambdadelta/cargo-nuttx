use std::ffi::OsString;

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

const DEFAULT_NUTTX_VERSION: &str = "13.0.0";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Specify the board to build for, using the name identifier NuttX uses
    ///
    /// For example, `--board raspberrypi-pico-2` will build for the Raspberry Pi Pico 2.
    #[arg(long)]
    pub board: String,
    /// Specify the base board specific config to use
    ///
    /// Most board have a config named `nsh`, which allows basic shell access
    #[arg(long, default_value = "nsh")]
    pub board_config: String,
    /// Clean the NuttX build directory before building (via `make distclean`)
    ///
    /// This is advisable when changing any configuration, so stale items don't interfere with the build, although this tool does attempt to mitigate the issue.
    #[arg(long, global = true)]
    pub clean: bool,
    /// Specify a package to build (default: the first viable package this tool finds)
    ///
    /// This is equivalent to the same native `cargo` flag
    #[arg(long, short, global = true)]
    pub package: Option<String>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build the crate and NuttX, linking them together into a firmware image
    Build {
        #[command(flatten)]
        nuttx_location: NuttxLocationArgs,
        /// Build using release profile optimizations
        ///
        /// This is equivalent to the same native `cargo` flag
        #[arg(long)]
        release: bool,
        /// Extra arguments to pass to `make` at the *build* stage of NuttX
        #[arg(last = true)]
        make_args: Vec<OsString>,
    },
}

#[derive(Clone)]
pub enum NuttxLocation {
    Path { root: Utf8PathBuf },
    Download { version: String },
}

#[derive(Args, Clone)]
#[group(multiple = false, required = true)]
pub struct NuttxLocationArgs {
    /// Path to the directory containing NuttX pre-extracted
    ///
    /// The structure expected should have a `nuttx` and an `apps` directory.
    #[arg(long)]
    path: Option<Utf8PathBuf>,
    /// Downloads NuttX from the official servers and extracts and uses it internally
    #[arg(long, num_args = 0..=1, default_missing_value = DEFAULT_NUTTX_VERSION)]
    download: Option<String>,
}

impl From<NuttxLocationArgs> for NuttxLocation {
    fn from(args: NuttxLocationArgs) -> Self {
        match (args.path, args.download) {
            (Some(root), _) => NuttxLocation::Path { root },
            (None, Some(version)) => NuttxLocation::Download { version },
            _ => unreachable!("clap group enforces exactly one"),
        }
    }
}
