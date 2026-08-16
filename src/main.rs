use std::{error::Error, fs};

use cargo_metadata::MetadataCommand;
use clap::Parser;
use convert_case::{Case, Casing};

use crate::{
    cargo::{find_package, find_potential_cargo_targets},
    cli::{Cli, Commands},
    metadata::get_nuttx_metadata,
    nuttx::build_nuttx,
};

mod cargo;
mod cli;
mod metadata;
mod nuttx;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("nuttx") {
        args.remove(1);
    }

    let cli = Cli::parse_from(args);
    let cargo_meta = MetadataCommand::new().exec()?;

    match cli.command {
        Commands::Build {
            release,
            nuttx_location,
            make_args,
        } => {
            let package = find_package(cli.package.as_deref(), &cargo_meta)?;

            let metadata = get_nuttx_metadata(&package)?;
            let board_metadata = metadata
                .boards
                .get(&cli.board)
                .ok_or_else(|| format!("No board metadata found for board {}", cli.board))?;

            let entrypoint_symbol = metadata
                .entrypoint_symbol
                .unwrap_or_else(|| format!("{}_main", package.name.as_str().to_case(Case::Snake)));

            let potential_targets =
                find_potential_cargo_targets(&package.targets).collect::<Vec<_>>();

            if potential_targets.len() != 1 {
                return Err(format!(
                    "Expected exactly one target, found these: {:?}
                        Make sure you set the target you want this tool to build as a `staticlib`",
                    potential_targets
                )
                .into());
            }

            let build_target = potential_targets[0];

            let config_fragment = if let Some(config_fragment) = metadata.config {
                fs::read_to_string(&config_fragment)?
            } else {
                "".to_string()
            };

            let board_config_path = metadata.board_config_directory.join(&cli.board);
            let board_config_fragment = if board_config_path.is_file() {
                fs::read_to_string(&board_config_path)?
            } else {
                "".to_string()
            };

            let nuttx_path = build_nuttx(
                &package.name,
                nuttx_location.into(),
                &cli.board,
                &cli.board_config,
                &config_fragment,
                &board_config_fragment,
                &build_target.name,
                &entrypoint_symbol,
                release,
                cli.clean,
                metadata.priority,
                metadata.stack_size,
                board_metadata,
                make_args,
            )?;

            let firmware_file_path = nuttx_path.join("nuttx").join(&board_metadata.firmware_file);
            let output_file = cargo_meta
                .target_directory
                .join("nuttx")
                .join(cli.board)
                .join(if release { "release" } else { "debug" })
                .join(&board_metadata.firmware_file);

            fs::create_dir_all(output_file.parent().unwrap())?;
            fs::copy(firmware_file_path, &output_file)?;

            println!("Firmware output to: {}", output_file);
        }
    }

    Ok(())
}
