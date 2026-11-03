use std::{collections::BTreeMap, error::Error};

use camino::Utf8PathBuf;
use cargo_metadata::Package;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Metadata {
    pub program_name: Option<String>,
    pub entrypoint_symbol: Option<String>,
    pub priority: u8,
    pub stack_size: u32,
    pub config: Option<Utf8PathBuf>,
    pub board_config_directory: Option<Utf8PathBuf>,
    pub nuttx_version: Option<String>,
    #[serde(default)]
    pub boards: BTreeMap<String, BoardSpecificMetadata>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct BoardSpecificMetadata {
    pub target: String,
    pub firmware_file: String,
    pub board_config: Option<String>,
    pub target_cpu: Option<String>,
    pub opt_level: Option<String>,
    pub runner: Option<Vec<String>>,
}

pub fn get_nuttx_metadata(package: &Package) -> Result<Metadata, Box<dyn Error>> {
    let raw_metadata = package.metadata.get("nuttx").ok_or_else(|| {
        format!(
            "Package {} has no [package.metadata.nuttx] table",
            package.name
        )
    })?;

    let mut metadata: Metadata = serde_json::from_value(raw_metadata.clone()).map_err(|error| {
        format!(
            "Invalid [package.metadata.nuttx] table in {}: {error}",
            package.manifest_path
        )
    })?;

    let manifest_directory = package
        .manifest_path
        .parent()
        .ok_or("Package manifest has no parent directory")?;
    metadata.config = metadata.config.map(|path| manifest_directory.join(path));
    metadata.board_config_directory = metadata
        .board_config_directory
        .map(|path| manifest_directory.join(path));

    Ok(metadata)
}
