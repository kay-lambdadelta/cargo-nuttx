use std::{error::Error, fs, process::ExitCode};

use camino::Utf8PathBuf;
use cargo_metadata::{MetadataCommand, Package};
use convert_case::{Case, Casing};

use crate::{
    cargo::{CargoInvocation, find_package, run_cargo},
    cli::{BuildArgs, Cli, Command, CommonArgs},
    metadata::{BoardSpecificMetadata, Metadata, get_nuttx_metadata},
    nuttx::{AppConfig, NuttxTree},
    process::run,
};

mod cargo;
mod cli;
mod metadata;
mod nuttx;
mod process;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Build(args) => build(&args).map(|_| ()),
        Command::Run(args) => run_firmware(&args),
        Command::Check(args) => check("check", &args),
        Command::Clippy(args) => check("clippy", &args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Project {
    package: Package,
    metadata: Metadata,
    board: String,
    board_config: String,
    target_directory: Utf8PathBuf,
    program_name: String,
    entrypoint_symbol: String,
    config_fragments: Vec<String>,
    tree: NuttxTree,
}

impl Project {
    fn load(args: &CommonArgs) -> Result<Self, Box<dyn Error>> {
        let cargo_meta = MetadataCommand::new().exec()?;
        let package = find_package(args.package.as_deref(), &cargo_meta)?.clone();
        let metadata = get_nuttx_metadata(&package)?;

        if !package.targets.iter().any(|target| target.is_staticlib()) {
            return Err(format!(
                "Package {} has no staticlib target; add `crate-type = [\"staticlib\"]` to its \
                 [lib] section",
                package.name
            )
            .into());
        }

        let known_boards = || {
            metadata
                .boards
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };

        let board = match &args.board {
            Some(board) => board.clone(),
            None if metadata.boards.len() == 1 => metadata.boards.keys().next().unwrap().clone(),
            None => {
                return Err(format!(
                    "Pick a board with --board (known boards: {})",
                    known_boards()
                )
                .into());
            }
        };

        let board_metadata = metadata.boards.get(&board).ok_or_else(|| {
            format!(
                "No [package.metadata.nuttx.boards.{board}] table in package {} (known boards: {})",
                package.name,
                known_boards()
            )
        })?;

        let board_config = args
            .board_config
            .clone()
            .or_else(|| board_metadata.board_config.clone())
            .unwrap_or_else(|| "nsh".to_string());

        let snake_case_name = package.name.as_str().to_case(Case::Snake);
        let program_name = metadata
            .program_name
            .clone()
            .unwrap_or_else(|| snake_case_name.clone());
        let entrypoint_symbol = metadata
            .entrypoint_symbol
            .clone()
            .unwrap_or_else(|| format!("{snake_case_name}_main"));

        let mut config_fragments = Vec::new();

        if let Some(config_path) = &metadata.config {
            config_fragments.push(read_config_fragment(config_path)?);
        }

        if let Some(board_config_directory) = &metadata.board_config_directory {
            if !board_config_directory.is_dir() {
                return Err(format!(
                    "board-config-directory {board_config_directory} does not exist"
                )
                .into());
            }

            let board_config_path = board_config_directory.join(&board);
            if board_config_path.is_file() {
                config_fragments.push(read_config_fragment(&board_config_path)?);
            }
        }

        let tree = NuttxTree::locate(
            args.nuttx.location(metadata.nuttx_version.as_deref()),
            &package.name,
            &board,
        )?;

        Ok(Self {
            package,
            metadata,
            board,
            board_config,
            target_directory: cargo_meta.target_directory,
            program_name,
            entrypoint_symbol,
            config_fragments,
            tree,
        })
    }

    fn board_metadata(&self) -> &BoardSpecificMetadata {
        &self.metadata.boards[&self.board]
    }

    fn app_config(&self) -> AppConfig<'_> {
        AppConfig {
            crate_name: &self.package.name,
            program_name: &self.program_name,
            entrypoint_symbol: &self.entrypoint_symbol,
            priority: self.metadata.priority,
            stack_size: self.metadata.stack_size,
            config_fragments: &self.config_fragments,
        }
    }

    fn configure(&self, clean: bool) -> Result<(), Box<dyn Error>> {
        self.tree
            .configure(&self.board, &self.board_config, &self.app_config(), clean)
    }

    fn cargo(
        &self,
        subcommand: &str,
        args: &CommonArgs,
    ) -> Result<Option<Utf8PathBuf>, Box<dyn Error>> {
        run_cargo(&CargoInvocation {
            subcommand,
            package: &self.package,
            release: args.release,
            jobs: args.jobs,
            board_metadata: self.board_metadata(),
            include_directories: &self.tree.include_directories(),
            extra_args: &args.cargo_args,
        })
    }
}

fn read_config_fragment(path: &Utf8PathBuf) -> Result<String, Box<dyn Error>> {
    fs::read_to_string(path)
        .map_err(|error| format!("Could not read NuttX config fragment {path}: {error}").into())
}

fn build(args: &BuildArgs) -> Result<(Project, Utf8PathBuf), Box<dyn Error>> {
    let common = &args.common;
    let project = Project::load(common)?;
    project.configure(common.clean)?;

    let staticlib_path = project
        .cargo("build", common)?
        .ok_or("cargo did not produce a static library")?;

    let app = project.app_config();
    project
        .tree
        .build(&app, &staticlib_path, common.jobs, &args.make_args)?;

    let output_directory = project
        .target_directory
        .join("nuttx")
        .join(&project.board)
        .join(if common.release { "release" } else { "debug" });
    fs::create_dir_all(&output_directory)?;

    let firmware_file = &project.board_metadata().firmware_file;
    for file_name in ["nuttx", firmware_file.as_str()] {
        let source = project.tree.nuttx.join(file_name);
        fs::copy(&source, output_directory.join(file_name))
            .map_err(|error| format!("Could not copy {source}: {error}"))?;
    }

    let firmware_path = output_directory.join(firmware_file);
    eprintln!("Firmware written to {firmware_path}");

    Ok((project, firmware_path))
}

fn run_firmware(args: &BuildArgs) -> Result<(), Box<dyn Error>> {
    let (project, firmware_path) = build(args)?;

    let Some([program, runner_args @ ..]) = project.board_metadata().runner.as_deref() else {
        return Err(format!(
            "No runner configured; set `runner` in [package.metadata.nuttx.boards.{}]",
            project.board
        )
        .into());
    };

    eprintln!("Running {program} {firmware_path}");
    run(std::process::Command::new(program)
        .args(runner_args)
        .arg(&firmware_path))
}

fn check(subcommand: &str, args: &CommonArgs) -> Result<(), Box<dyn Error>> {
    let project = Project::load(args)?;
    project.configure(args.clean)?;
    project.cargo(subcommand, args)?;

    Ok(())
}
