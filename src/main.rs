use std::{error::Error, ffi::OsString, fs, process::ExitCode};

use camino::Utf8PathBuf;
use cargo_metadata::{MetadataCommand, Package};
use convert_case::{Case, Casing};

use crate::{
    cargo::{CargoInvocation, CrateSelection, find_package, run_cargo},
    cli::{BuildArgs, Cli, Command, CommonArgs},
    layout::{BoardFiles, GlobalFiles},
    metadata::{BoardSpecificMetadata, Metadata, get_nuttx_metadata},
    nuttx::{AppConfig, NuttxTree, write_if_changed},
    patch::collect_patches,
    process::run,
};

mod cargo;
mod cli;
mod layout;
mod metadata;
mod nuttx;
mod patch;
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
    target: String,
    target_directory: Utf8PathBuf,
    program_name: String,
    entrypoint_symbol: String,
    config_fragments: Vec<String>,
    board_files: BoardFiles,
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

        let global_files = GlobalFiles::locate(metadata.directory.as_deref());
        let board_files = BoardFiles::locate(metadata.directory.as_deref(), &board);

        let target = resolve_target(&board, board_metadata, &board_files)?;

        let config_fragments = [&global_files.config, &board_files.config]
            .into_iter()
            .flatten()
            .map(read_config_fragment)
            .collect::<Result<Vec<_>, _>>()?;

        let patch_directories = [&global_files.patch_directory, &board_files.patch_directory]
            .into_iter()
            .flatten()
            .map(Utf8PathBuf::as_path)
            .collect::<Vec<_>>();
        let patches = collect_patches(&patch_directories)?;

        let tree = NuttxTree::locate(
            args.nuttx.location(metadata.nuttx_version.as_deref()),
            &package.name,
            &board,
            &patches,
        )?;

        Ok(Self {
            package,
            metadata,
            board,
            board_config,
            target,
            target_directory: cargo_meta.target_directory,
            program_name,
            entrypoint_symbol,
            config_fragments,
            board_files,
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
            .configure(&self.board_specification()?, &self.app_config(), clean)
    }

    fn board_specification(&self) -> Result<String, Box<dyn Error>> {
        let Some(custom_board) = &self.board_files.custom_board else {
            return Ok(format!("{}:{}", self.board, self.board_config));
        };

        let source_directory = custom_board.join("configs").join(&self.board_config);
        let generated_directory = self
            .target_directory
            .join("nuttx")
            .join(&self.board)
            .join("configs")
            .join(&self.board_config);
        fs::create_dir_all(&generated_directory)?;

        let mut defconfig = fs::read_to_string(source_directory.join("defconfig"))
            .map_err(|error| format!("Could not read defconfig in {source_directory}: {error}"))?;

        defconfig.push_str(&format!(
            "\nCONFIG_ARCH_BOARD_CUSTOM_DIR=\"{custom_board}\"\n# \
             CONFIG_ARCH_BOARD_CUSTOM_DIR_RELPATH is not set\n"
        ));

        let defconfig_path = generated_directory.join("defconfig");
        if fs::read_to_string(&defconfig_path).ok().as_ref() != Some(&defconfig) {
            fs::write(&defconfig_path, &defconfig)?;

            self.tree.invalidate_configuration()?;
        }

        let make_defs = fs::read_to_string(custom_board.join("scripts").join("Make.defs"))
            .map_err(|error| format!("Could not read Make.defs in {custom_board}: {error}"))?;
        write_if_changed(&generated_directory.join("Make.defs"), &make_defs)?;

        Ok(generated_directory.into_string())
    }

    fn cargo(
        &self,
        subcommand: &str,
        args: &CommonArgs,
    ) -> Result<Option<Utf8PathBuf>, Box<dyn Error>> {
        run_cargo(&CargoInvocation {
            subcommand,
            selection: CrateSelection::WorkspacePackage(&self.package),
            build_std: "std,panic_abort",
            target: &self.target,
            release: args.release,
            jobs: args.jobs,
            board_metadata: self.board_metadata(),
            include_directories: &self.tree.include_directories(),
            extra_args: &args.cargo_args,
        })
    }

    fn linker_script_make_args(&self) -> Result<Vec<OsString>, Box<dyn Error>> {
        let Some(linker_script) = &self.board_files.linker_script else {
            return Ok(Vec::new());
        };

        let file_name = linker_script
            .file_name()
            .ok_or_else(|| format!("Linker script {linker_script} has no file name"))?;
        let contents = fs::read_to_string(linker_script)
            .map_err(|error| format!("Could not read linker script {linker_script}: {error}"))?;

        let generated_directory = self.target_directory.join("nuttx").join(&self.board);
        fs::create_dir_all(&generated_directory)?;
        let generated_path = generated_directory.join(file_name);
        write_if_changed(&generated_path, &contents)?;

        Ok(vec![format!("ARCHSCRIPT={generated_path}").into()])
    }

    fn board_crate(&self, args: &CommonArgs) -> Result<Option<Utf8PathBuf>, Box<dyn Error>> {
        let Some(board_crate) = &self.board_files.board_crate else {
            return Ok(None);
        };

        let staticlib_path = run_cargo(&CargoInvocation {
            subcommand: "build",
            selection: CrateSelection::Manifest(board_crate),
            build_std: "core",
            target: &self.target,
            release: args.release,
            jobs: args.jobs,
            board_metadata: self.board_metadata(),
            include_directories: &self.tree.include_directories(),
            extra_args: &[],
        })?
        .ok_or_else(|| format!("Board crate {board_crate} did not produce a static library"))?;

        Ok(Some(staticlib_path))
    }
}

fn resolve_target(
    board: &str,
    board_metadata: &BoardSpecificMetadata,
    board_files: &BoardFiles,
) -> Result<String, Box<dyn Error>> {
    match (&board_metadata.target, &board_files.target_specification) {
        (Some(target), None) if target.ends_with(".json") => Err(format!(
            "Board {board} names a target specification file, instead of a target triplet. Move it to \
             boards/{board}/target.json and remove `target`"
        )
        .into()),
        (Some(target), None) => Ok(target.clone()),
        (None, Some(target_specification)) => Ok(target_specification.to_string()),
        (Some(_), Some(target_specification)) => {
            Err(format!("Board {board} sets `target` and has {target_specification}").into())
        }
        (None, None) => {
            Err(format!("Board {board} has no `target` and no boards/{board}/target.json").into())
        }
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
    let board_staticlib_path = project.board_crate(common)?;

    let staticlib_paths: Vec<_> = [Some(&staticlib_path), board_staticlib_path.as_ref()]
        .into_iter()
        .flatten()
        .map(|path| path.as_path())
        .collect();

    let mut make_args = project.linker_script_make_args()?;
    make_args.extend(args.make_args.iter().cloned());

    let app = project.app_config();
    project
        .tree
        .build(&app, &staticlib_paths, common.jobs, &make_args)?;

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
