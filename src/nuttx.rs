use std::{
    error::Error, ffi::OsString, fs, io, num::NonZeroUsize, process::Command,
    thread::available_parallelism, time::SystemTime,
};

use camino::{Utf8Path, Utf8PathBuf};
use convert_case::{Case, Casing};
use dirs::cache_dir;
use flate2::read::GzDecoder;

use crate::{cli::NuttxLocation, process::run_build_step};

const NUTTX_MIRRORS: [&str; 2] = [
    "https://dlcdn.apache.org/nuttx",
    "https://archive.apache.org/dist/nuttx",
];

const STATE_DIRECTORY: &str = concat!(".", env!("CARGO_PKG_NAME"));

pub struct AppConfig<'a> {
    pub crate_name: &'a str,
    pub program_name: &'a str,
    pub entrypoint_symbol: &'a str,
    pub priority: u8,
    pub stack_size: u32,
    pub config_fragments: &'a [String],
}

impl AppConfig<'_> {
    fn config_symbol(&self) -> String {
        format!("CARGO_NUTTX_{}", self.crate_name.to_case(Case::UpperSnake))
    }
}

pub struct NuttxTree {
    pub nuttx: Utf8PathBuf,
    pub apps: Utf8PathBuf,
}

impl NuttxTree {
    pub fn locate(
        location: NuttxLocation,
        crate_name: &str,
        board: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let root = match location {
            NuttxLocation::Path { root } => root,
            NuttxLocation::Download { version } => {
                let root = Utf8PathBuf::try_from(
                    cache_dir()
                        .ok_or("Could not locate cache dir for your os")?
                        .join(env!("CARGO_CRATE_NAME"))
                        .join("nuttx")
                        .join(&version)
                        .join(crate_name)
                        .join(board),
                )?;

                download_nuttx(&version, &root)?;

                root
            }
        };

        let tree = Self {
            nuttx: root.join("nuttx"),
            apps: root.join("apps"),
        };

        if !tree.nuttx.is_dir() || !tree.apps.is_dir() {
            return Err(format!(
                "{root} is not a NuttX workspace (expected `nuttx` and `apps` directories)"
            )
            .into());
        }

        Ok(tree)
    }

    pub fn include_directories(&self) -> Vec<Utf8PathBuf> {
        vec![
            self.nuttx.join("include"),
            self.nuttx.join("include").join("arch"),
        ]
    }

    pub fn configure(
        &self,
        board: &str,
        board_config: &str,
        app: &AppConfig,
        clean: bool,
    ) -> Result<(), Box<dyn Error>> {
        let state_directory = self.nuttx.join(STATE_DIRECTORY);
        let configured_marker = state_directory.join("configured");
        let pristine_config_path = state_directory.join("pristine.config");
        let applied_config_path = state_directory.join("applied.config");
        let config_path = self.nuttx.join(".config");
        let board_spec = format!("{board}:{board_config}");

        let already_configured = !clean
            && pristine_config_path.is_file()
            && fs::read_to_string(&configured_marker).is_ok_and(|marker| marker == board_spec);

        if !already_configured {
            if config_path.exists() {
                eprintln!("Cleaning NuttX");
                run_build_step(
                    Command::new("make")
                        .current_dir(&self.nuttx)
                        .arg("distclean"),
                )?;
            }

            remove_dir_if_exists(&state_directory)?;
        }

        self.install_app(app, None)?;

        if !already_configured {
            eprintln!("Configuring NuttX for {board_spec}");
            run_build_step(
                Command::new("./tools/configure.sh")
                    .current_dir(&self.nuttx)
                    .arg(&board_spec),
            )?;

            fs::create_dir_all(&state_directory)?;
            fs::copy(&config_path, &pristine_config_path)?;
            fs::write(&configured_marker, &board_spec)?;
        }

        // Inject the config fragments for the crate
        let mut config = fs::read_to_string(&pristine_config_path)?;
        config.push('\n');
        config.push_str(&form_config_overrides(app));
        for fragment in app.config_fragments {
            config.push('\n');
            config.push_str(fragment);
        }
        config.push('\n');

        let applied_config = fs::read_to_string(&applied_config_path).ok();
        if !config_path.is_file() || applied_config.as_ref() != Some(&config) {
            if applied_config.is_some() {
                eprintln!("Cleaning NuttX build outputs after a config change");
                run_build_step(Command::new("make").current_dir(&self.nuttx).arg("clean"))?;
            }

            fs::write(&config_path, &config)?;
            run_build_step(
                Command::new("make")
                    .current_dir(&self.nuttx)
                    .arg("olddefconfig"),
            )?;
            fs::write(&applied_config_path, &config)?;
        }

        // Setup the headers
        self.invalidate_openlibm_placeholder()?;
        run_build_step(Command::new("make").current_dir(&self.nuttx).arg("context"))?;
        self.write_openlibm_placeholder()?;

        Ok(())
    }

    // FIXME: Report the terrible openlibm hack upstream
    //
    // This is needed so that rebuilds can happen without a call to clean

    fn openlibm_paths(&self) -> Option<(Utf8PathBuf, Utf8PathBuf, String)> {
        let openlibm_directory = self.nuttx.join("libs").join("libm").join("openlibm");
        let make_defs = fs::read_to_string(openlibm_directory.join("Make.defs")).ok()?;
        let version = make_defs
            .lines()
            .find_map(|line| line.strip_prefix("OPENLIBM_VERSION="))?
            .trim()
            .to_string();

        let sources = openlibm_directory.join("openlibm");
        let placeholder = self
            .nuttx
            .join("libs")
            .join("libm")
            .join(format!("openlibm-{version}.zip"));

        Some((sources, placeholder, version))
    }

    fn invalidate_openlibm_placeholder(&self) -> Result<(), Box<dyn Error>> {
        let Some((sources, placeholder, version)) = self.openlibm_paths() else {
            return Ok(());
        };

        if sources.join(format!("openlibm-{version}")).exists() {
            remove_dir_if_exists(&sources)?;
        }

        if !sources.is_dir() && placeholder.is_file() {
            fs::remove_file(&placeholder)?;
        }

        Ok(())
    }

    fn write_openlibm_placeholder(&self) -> Result<(), Box<dyn Error>> {
        let Some((sources, placeholder, _)) = self.openlibm_paths() else {
            return Ok(());
        };

        if sources.is_dir() && !sources.join(".git").exists() && !placeholder.exists() {
            fs::File::create(&placeholder)?.set_modified(SystemTime::UNIX_EPOCH)?;
        }

        Ok(())
    }

    pub fn build(
        &self,
        app: &AppConfig,
        staticlib_path: &Utf8Path,
        jobs: Option<NonZeroUsize>,
        make_args: &[OsString],
    ) -> Result<(), Box<dyn Error>> {
        self.install_app(app, Some(staticlib_path))?;

        let jobs = match jobs {
            Some(jobs) => jobs,
            None => available_parallelism()?,
        };

        eprintln!("Building NuttX");
        run_build_step(
            Command::new("make")
                .current_dir(&self.nuttx)
                .arg(format!("--jobs={jobs}"))
                .args(make_args),
        )
    }

    fn install_app(
        &self,
        app: &AppConfig,
        staticlib_path: Option<&Utf8Path>,
    ) -> Result<(), Box<dyn Error>> {
        let external_directory = self.apps.join("external");
        let shim_destination = external_directory.join(app.crate_name);
        fs::create_dir_all(&shim_destination)?;

        let makefile = external_directory.join("Makefile");
        if !makefile.is_file() {
            fs::write(
                &makefile,
                "MENUDESC = \"External Apps\"\ninclude $(APPDIR)/Directory.mk\n",
            )?;
        }

        let make_defs = external_directory.join("Make.defs");
        if !make_defs.is_file() {
            fs::write(
                &make_defs,
                "include $(wildcard $(APPDIR)/external/*/Make.defs)\n",
            )?;
        }

        write_if_changed(&shim_destination.join("Kconfig"), &form_kconfig(app))?;
        write_if_changed(
            &shim_destination.join("Make.defs"),
            &form_make_defs(app, staticlib_path),
        )?;
        write_if_changed(&shim_destination.join("Makefile"), &form_makefile(app))?;

        Ok(())
    }
}

fn form_kconfig(app: &AppConfig) -> String {
    let symbol = app.config_symbol();
    let program_name = app.program_name;
    let priority = app.priority;
    let stack_size = app.stack_size;

    format!(
        "
config {symbol}
    tristate \"A rust application\"
    default n

if {symbol}
    config {symbol}_PROGNAME
        string \"Program name\"
        default \"{program_name}\"

    config {symbol}_PRIORITY
        int \"Task priority\"
        default {priority}

    config {symbol}_STACKSIZE
        int \"Stack size\"
        default {stack_size}
endif
"
    )
}

fn form_make_defs(app: &AppConfig, staticlib_path: Option<&Utf8Path>) -> String {
    let symbol = app.config_symbol();
    let crate_name = app.crate_name;
    let extra_libs = staticlib_path
        .map(|path| format!("    EXTRA_LIBS += \"{path}\"\n"))
        .unwrap_or_default();

    format!(
        "
ifneq ($(CONFIG_{symbol}),)
    CONFIGURED_APPS += $(APPDIR)/external/{crate_name}
{extra_libs}endif
"
    )
}

fn form_makefile(app: &AppConfig) -> String {
    let symbol = app.config_symbol();

    format!(
        "
include $(APPDIR)/Make.defs

PROGNAME  = $(CONFIG_{symbol}_PROGNAME)
PRIORITY  = $(CONFIG_{symbol}_PRIORITY)
STACKSIZE = $(CONFIG_{symbol}_STACKSIZE)
MODULE    = $(CONFIG_{symbol})

include $(APPDIR)/Application.mk
"
    )
}

fn form_config_overrides(app: &AppConfig) -> String {
    let symbol = app.config_symbol();
    let program_name = app.program_name;
    let priority = app.priority;
    let stack_size = app.stack_size;
    let entrypoint_symbol = app.entrypoint_symbol;

    format!(
        "CONFIG_{symbol}=y
CONFIG_{symbol}_PROGNAME=\"{program_name}\"
CONFIG_{symbol}_PRIORITY={priority}
CONFIG_{symbol}_STACKSIZE={stack_size}
CONFIG_INIT_ENTRYPOINT=\"{entrypoint_symbol}\"
CONFIG_INIT_ENTRYNAME=\"{entrypoint_symbol}\"
CONFIG_LIBM_OPENLIBM=y
"
    )
}

fn download_nuttx(version: &str, destination: &Utf8Path) -> Result<(), Box<dyn Error>> {
    if destination.join("nuttx").is_dir() && destination.join("apps").is_dir() {
        return Ok(());
    }

    let staging = Utf8PathBuf::from(format!("{destination}.partial"));
    remove_dir_if_exists(&staging)?;
    fs::create_dir_all(&staging)?;

    for archive in ["apache-nuttx", "apache-nuttx-apps"] {
        eprintln!("Downloading {archive}-{version}");

        let body = fetch_archive(version, archive)?.into_body();
        let mut tar_archive = tar::Archive::new(GzDecoder::new(body.into_reader()));
        tar_archive.unpack(&staging)?;
    }

    remove_dir_if_exists(destination)?;
    fs::rename(&staging, destination)?;

    Ok(())
}

fn fetch_archive(
    version: &str,
    archive: &str,
) -> Result<ureq::http::Response<ureq::Body>, Box<dyn Error>> {
    for mirror in NUTTX_MIRRORS {
        let url = format!("{mirror}/{version}/{archive}-{version}.tar.gz");

        match ureq::get(&url).call() {
            Ok(response) => return Ok(response),
            Err(ureq::Error::StatusCode(404)) => continue,
            Err(error) => return Err(format!("Could not download {url}: {error}").into()),
        }
    }

    Err(format!("NuttX {version} was not found on any Apache mirror").into())
}

fn write_if_changed(path: &Utf8Path, contents: &str) -> io::Result<()> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }

    fs::write(path, contents)
}

fn remove_dir_if_exists(path: &Utf8Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}
