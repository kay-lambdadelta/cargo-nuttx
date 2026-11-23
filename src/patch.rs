use std::{error::Error, fs};

use blake3::Hasher;
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use diffy::patch_set::{FileOperation, ParseOptions, PatchSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repository {
    Nuttx,
    Apps,
}

impl Repository {
    const ALL: [Self; 2] = [Self::Nuttx, Self::Apps];

    pub fn directory_name(self) -> &'static str {
        match self {
            Self::Nuttx => "nuttx",
            Self::Apps => "apps",
        }
    }
}

pub struct PatchFile {
    pub repository: Repository,
    pub path: Utf8PathBuf,
    pub contents: String,
}

pub fn collect_patches(directories: &[&Utf8Path]) -> Result<Vec<PatchFile>, Box<dyn Error>> {
    let mut patches = Vec::new();

    for directory in directories {
        if !directory.is_dir() {
            return Err(format!("Patch directory {directory} does not exist").into());
        }

        for repository in Repository::ALL {
            let repository_directory = directory.join(repository.directory_name());
            if !repository_directory.is_dir() {
                continue;
            }

            let mut paths = repository_directory
                .read_dir_utf8()?
                .map(|entry| entry.map(|entry| entry.into_path()))
                .collect::<Result<Vec<_>, _>>()?;
            paths.retain(|path| path.extension() == Some("patch") && path.is_file());
            paths.sort();

            for path in paths {
                let contents = fs::read_to_string(&path)
                    .map_err(|error| format!("Could not read patch {path}: {error}"))?;

                patches.push(PatchFile {
                    repository,
                    path,
                    contents,
                });
            }
        }
    }

    Ok(patches)
}

pub fn fingerprint(patches: &[PatchFile]) -> String {
    let mut hasher = Hasher::default();

    for (index, patch_file) in patches.iter().enumerate() {
        hasher.update(&(index as u32).to_le_bytes());
        hasher.update(patch_file.path.file_name().unwrap_or_default().as_bytes());
        hasher.update(patch_file.repository.directory_name().as_bytes());
        hasher.update(patch_file.contents.as_bytes());
        hasher.update(&(patch_file.contents.len() as u32).to_le_bytes());
    }

    hasher.finalize().to_string()
}

pub fn apply_patches(root: &Utf8Path, patches: &[PatchFile]) -> Result<(), Box<dyn Error>> {
    for patch in patches {
        eprintln!("Applying {}", patch.path);

        let tree = root.join(patch.repository.directory_name());
        apply_patch(&tree, &patch.contents)
            .map_err(|error| format!("Could not apply {}: {error}", patch.path))?;
    }

    Ok(())
}

fn apply_patch(tree: &Utf8Path, contents: &str) -> Result<(), Box<dyn Error>> {
    for file_patch in PatchSet::parse(contents, ParseOptions::gitdiff()) {
        let file_patch = file_patch.map_err(|error| format!("parse error: {error}"))?;
        let text_patch = file_patch
            .patch()
            .as_text()
            .ok_or("binary patches are not supported")?;

        match file_patch.operation().strip_prefix(1) {
            FileOperation::Modify { original, modified } if original == modified => {
                let path = resolve(tree, &original)?;
                let base = fs::read_to_string(&path)
                    .map_err(|error| format!("could not read {path}: {error}"))?;
                let patched = diffy::apply(&base, text_patch)
                    .map_err(|error| format!("{path} does not match: {error}"))?;

                fs::write(&path, patched)?;
            }
            FileOperation::Create(path) => {
                let path = resolve(tree, &path)?;
                if path.exists() {
                    return Err(format!("{path} already exists").into());
                }

                let created = diffy::apply("", text_patch)
                    .map_err(|error| format!("{path} could not be created: {error}"))?;

                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, created)?;
            }
            FileOperation::Delete(path) => {
                let path = resolve(tree, &path)?;
                let base = fs::read_to_string(&path)
                    .map_err(|error| format!("could not read {path}: {error}"))?;
                let remaining = diffy::apply(&base, text_patch)
                    .map_err(|error| format!("{path} does not match: {error}"))?;

                if !remaining.is_empty() {
                    return Err(format!("{path} does not match the deletion").into());
                }

                fs::remove_file(&path)?;
            }
            other => {
                return Err(format!("unsupported file operation {other:?}").into());
            }
        }
    }

    Ok(())
}

fn resolve(tree: &Utf8Path, relative: &str) -> Result<Utf8PathBuf, Box<dyn Error>> {
    let relative = Utf8Path::new(relative);

    if !relative
        .components()
        .all(|component| matches!(component, Utf8Component::Normal(_)))
    {
        return Err(format!("{relative} points outside the tree").into());
    }

    Ok(tree.join(relative))
}
