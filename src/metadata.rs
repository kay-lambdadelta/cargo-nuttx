use std::{collections::HashMap, error::Error};

use camino::Utf8PathBuf;
use cargo_metadata::Package;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    #[serde(rename = "program-name")]
    pub program_name: Option<String>,
    #[serde(rename = "entrypoint-symbol")]
    pub entrypoint_symbol: Option<String>,
    pub priority: u8,
    #[serde(rename = "stack-size")]
    pub stack_size: u32,
    pub config: Option<Utf8PathBuf>,
    #[serde(default)]
    pub boards: HashMap<String, BoardSpecificMetadata>,
    #[serde(rename = "board-config-directory")]
    pub board_config_directory: Utf8PathBuf,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct BoardSpecificMetadata {
    pub target: String,
    #[serde(rename = "firmware-file")]
    pub firmware_file: String,
    #[serde(rename = "target-cpu")]
    pub target_cpu: Option<String>,
    #[serde(rename = "opt-level")]
    pub opt_level: Option<String>,
}

pub fn get_nuttx_metadata(package: &Package) -> Result<Metadata, Box<dyn Error>> {
    let raw_metadata = package.metadata.get("nuttx").ok_or_else(|| {
        format!(
            "Package {} has no [package.metadata.nuttx] table",
            package.name
        )
    })?;

    Ok(serde_json::from_value(raw_metadata.clone())?)
}
