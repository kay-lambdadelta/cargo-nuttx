use std::{
    error::Error,
    io::BufReader,
    process::{Command, Stdio},
};

use camino::Utf8PathBuf;
use cargo_metadata::{Message, Metadata, Package, Target};

use crate::metadata::BoardSpecificMetadata;

pub fn find_package(
    explicit_name: Option<&str>,
    cargo_meta: &Metadata,
) -> Result<Package, Box<dyn Error>> {
    if let Some(name) = explicit_name {
        return cargo_meta
            .packages
            .iter()
            .find(|package| package.name.as_str() == name)
            .ok_or_else(|| "No such package in workspace: {name}".into())
            .cloned();
    }

    let working_directory = std::env::current_dir()?;
    cargo_meta
        .workspace_packages()
        .into_iter()
        .filter(|p| {
            let manifest_dir = p.manifest_path.parent().unwrap();

            working_directory.starts_with(manifest_dir)
        })
        .max_by_key(|p| p.manifest_path.as_str().len())
        .cloned()
        .ok_or_else(|| "Could not determine current package; pass the -p flag".into())
}

pub fn find_potential_cargo_targets<'a>(
    targets: impl IntoIterator<Item = &'a Target> + 'a,
) -> impl Iterator<Item = &'a Target> + 'a {
    targets.into_iter().filter(|target| target.is_staticlib())
}

pub fn build_cargo(
    crate_name: &str,
    build_target_name: &str,
    release: bool,
    board_metadata: &BoardSpecificMetadata,
    include_directories: impl IntoIterator<Item = Utf8PathBuf>,
) -> Result<Utf8PathBuf, Box<dyn Error>> {
    let include_env_var_value = include_directories
        .into_iter()
        .map(|paths| paths.to_string())
        .collect::<Vec<_>>()
        .join(":");

    // Get the old rust flags and add some knobs the user can directly set in the config file
    let mut rustflags: Vec<_> = std::env::var("RUSTFLAGS")
        .map(|flags| flags.split(" ").map(|flag| flag.to_string()).collect())
        .unwrap_or_default();

    if let Some(target_cpu) = &board_metadata.target_cpu {
        rustflags.push(format!("-Ctarget-cpu={}", target_cpu));
    }

    if let Some(opt_level) = &board_metadata.opt_level {
        rustflags.push(format!("-Copt-level={}", opt_level));
    }

    let mut args = vec![
        "build",
        "--message-format=json-render-diagnostics",
        "-Zunstable-options",
        "-Zbuild-std=std,panic_abort",
        "-Zjson-target-spec",
        "--target",
        &board_metadata.target,
        "--package",
        crate_name,
    ];

    if release {
        args.push("--release");
    };

    let mut child = Command::new("cargo")
        .args(args)
        .env("NUTTX_INCLUDE_DIR", include_env_var_value)
        .env("RUSTFLAGS", rustflags.join(" "))
        .stdout(Stdio::piped())
        .spawn()?;

    let reader = BufReader::new(child.stdout.take().ok_or("No stdout")?);
    let mut staticlib_path = None;

    for message in Message::parse_stream(reader) {
        if let Ok(Message::CompilerArtifact(artifact)) = message
            && artifact.target.name == build_target_name
            && artifact.target.is_staticlib()
            && let Some(path) = artifact.filenames.first()
        {
            staticlib_path = Some(path.to_path_buf());
        }
    }

    child.wait()?;

    Ok(staticlib_path.ok_or("No staticlib produced!")?)
}
