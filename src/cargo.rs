use std::{
    env,
    error::Error,
    io::BufReader,
    num::NonZero,
    process::{Command, Stdio},
};

use camino::Utf8PathBuf;
use cargo_metadata::{Message, Metadata, Package};

use crate::metadata::BoardSpecificMetadata;

pub fn find_package<'a>(
    explicit_name: Option<&str>,
    cargo_meta: &'a Metadata,
) -> Result<&'a Package, Box<dyn Error>> {
    let workspace_packages = cargo_meta.workspace_packages();

    if let Some(name) = explicit_name {
        return workspace_packages
            .into_iter()
            .find(|package| package.name.as_str() == name)
            .ok_or_else(|| format!("No package named `{name}` in this workspace").into());
    }

    let nuttx_packages: Vec<_> = workspace_packages
        .into_iter()
        .filter(|package| package.metadata.get("nuttx").is_some())
        .collect();

    let working_directory = env::current_dir()?;
    if let Some(package) = nuttx_packages
        .iter()
        .filter(|package| {
            package
                .manifest_path
                .parent()
                .is_some_and(|manifest_directory| working_directory.starts_with(manifest_directory))
        })
        .max_by_key(|package| package.manifest_path.as_str().len())
    {
        return Ok(package);
    }

    match nuttx_packages.as_slice() {
        [package] => Ok(package),
        [] => Err("No package in this workspace has a [package.metadata.nuttx] table".into()),
        _ => Err("Several packages in this workspace have NuttX metadata; pick one with -p".into()),
    }
}

pub struct CargoInvocation<'a> {
    pub subcommand: &'a str,
    pub package: &'a Package,
    pub release: bool,
    pub jobs: Option<NonZero<usize>>,
    pub board_metadata: &'a BoardSpecificMetadata,
    pub include_directories: &'a [Utf8PathBuf],
    pub extra_args: &'a [String],
}

pub fn run_cargo(invocation: &CargoInvocation) -> Result<Option<Utf8PathBuf>, Box<dyn Error>> {
    let include_env_var_value = invocation
        .include_directories
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(":");

    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .arg(invocation.subcommand)
        .args([
            "--message-format=json-render-diagnostics",
            "-Zunstable-options",
            "-Zbuild-std=std,panic_abort",
            "-Zjson-target-spec",
            "--target",
            &invocation.board_metadata.target,
            "--package",
            &invocation.package.name,
        ])
        .env("NUTTX_INCLUDE_DIR", include_env_var_value)
        .stdout(Stdio::piped());

    if invocation.subcommand == "build" {
        command.arg("--lib");
    }

    if invocation.release {
        command.arg("--release");
    }

    if let Some(jobs) = invocation.jobs {
        command.arg(format!("--jobs={jobs}"));
    }

    add_rustflags(&mut command, invocation.board_metadata);
    command.args(invocation.extra_args);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not run cargo: {error}"))?;

    let reader = BufReader::new(child.stdout.take().ok_or("No stdout")?);
    let mut staticlib_path = None;

    for message in Message::parse_stream(reader) {
        if let Message::CompilerArtifact(artifact) = message?
            && artifact.package_id == invocation.package.id
            && artifact.target.is_staticlib()
            && let Some(path) = artifact
                .filenames
                .into_iter()
                .find(|path| path.extension() == Some("a"))
        {
            staticlib_path = Some(path);
        }
    }

    let exit_status = child.wait()?;
    if !exit_status.success() {
        return Err(format!("cargo {} failed ({exit_status})", invocation.subcommand).into());
    }

    Ok(staticlib_path)
}

fn add_rustflags(command: &mut Command, board_metadata: &BoardSpecificMetadata) {
    let mut board_flags = Vec::new();

    if let Some(target_cpu) = &board_metadata.target_cpu {
        board_flags.push(format!("-Ctarget-cpu={target_cpu}"));
    }

    if let Some(opt_level) = &board_metadata.opt_level {
        board_flags.push(format!("-Copt-level={opt_level}"));
    }

    if board_flags.is_empty() {
        return;
    }

    if let Ok(encoded) = env::var("CARGO_ENCODED_RUSTFLAGS") {
        let mut flags: Vec<_> = encoded
            .split('\x1f')
            .filter(|flag| !flag.is_empty())
            .map(str::to_string)
            .collect();
        flags.extend(board_flags);
        command.env("CARGO_ENCODED_RUSTFLAGS", flags.join("\x1f"));
    } else {
        let mut flags: Vec<_> = env::var("RUSTFLAGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        flags.extend(board_flags);
        command.env("RUSTFLAGS", flags.join(" "));
    }
}
