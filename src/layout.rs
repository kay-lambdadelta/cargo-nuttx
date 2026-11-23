use camino::{Utf8Path, Utf8PathBuf};

pub struct GlobalFiles {
    pub config: Option<Utf8PathBuf>,
    pub patch_directory: Option<Utf8PathBuf>,
}

pub struct BoardFiles {
    pub config: Option<Utf8PathBuf>,
    pub custom_board: Option<Utf8PathBuf>,
    pub board_crate: Option<Utf8PathBuf>,
    pub linker_script: Option<Utf8PathBuf>,
    pub patch_directory: Option<Utf8PathBuf>,
    pub target_specification: Option<Utf8PathBuf>,
}

impl GlobalFiles {
    pub fn locate(directory: Option<&Utf8Path>) -> Self {
        Self {
            config: directory.and_then(|directory| existing_file(directory.join("config"))),
            patch_directory: directory
                .and_then(|directory| existing_directory(directory.join("patches"))),
        }
    }
}

impl BoardFiles {
    pub fn locate(directory: Option<&Utf8Path>, board: &str) -> Self {
        let board_directory = directory
            .map(|directory| directory.join("boards").join(board))
            .and_then(existing_directory);

        let file = |name| {
            board_directory
                .as_ref()
                .and_then(|board_directory| existing_file(board_directory.join(name)))
        };

        let subdirectory = |name| {
            board_directory
                .as_ref()
                .and_then(|board_directory| existing_directory(board_directory.join(name)))
        };

        let custom_board = subdirectory("custom");
        let board_crate = custom_board
            .as_ref()
            .map(|custom_board| custom_board.join("rust"))
            .filter(|board_crate| board_crate.join("Cargo.toml").is_file());

        Self {
            config: file("config"),
            custom_board,
            board_crate,
            linker_script: file("linker.ld"),
            patch_directory: subdirectory("patches"),
            target_specification: file("target.json"),
        }
    }
}

fn existing_file(path: Utf8PathBuf) -> Option<Utf8PathBuf> {
    path.is_file().then_some(path)
}

fn existing_directory(path: Utf8PathBuf) -> Option<Utf8PathBuf> {
    path.is_dir().then_some(path)
}
