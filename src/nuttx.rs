use std::{
    error::Error,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    process::Command,
};

use camino::{Utf8Path, Utf8PathBuf};
use convert_case::{Case, Casing};
use dirs::cache_dir;
use flate2::read::GzDecoder;

use crate::{cargo::build_cargo, cli::NuttxLocation, metadata::BoardSpecificMetadata};

fn nuttx_url(version: &str) -> String {
    format!(
        "https://dlcdn.apache.org/nuttx/{}/apache-nuttx-{}.tar.gz",
        version, version
    )
}

fn nuttx_apps_url(version: &str) -> String {
    format!(
        "https://dlcdn.apache.org/nuttx/{}/apache-nuttx-apps-{}.tar.gz",
        version, version
    )
}

fn form_kconfig(crate_name: &str, priority: u8, stack_size: u32) -> String {
    let uppercase_crate_name = crate_name.to_case(Case::UpperSnake);
    let crate_name = crate_name.to_case(Case::Snake);

    format!(
        "
config CARGO_NUTTX_{uppercase_crate_name}
    tristate \"A rust application\"
    default n

if CARGO_NUTTX_{uppercase_crate_name}
    config CARGO_NUTTX_{uppercase_crate_name}_PROGNAME
        string \"Program name\"
        default \"{crate_name}\"

    config CARGO_NUTTX_{uppercase_crate_name}_PRIORITY
        int \"Task priority\"
        default {priority}

    config CARGO_NUTTX_{uppercase_crate_name}_STACKSIZE
        int \"Stack size\"
        default {stack_size}
endif
       "
    )
}

fn form_make_defs(crate_name: &str, staticlib_path: &Utf8Path) -> String {
    let uppercase_crate_name = crate_name.to_case(Case::UpperSnake);

    format!(
        "
ifneq ($(CONFIG_CARGO_NUTTX_{uppercase_crate_name}),)
    CONFIGURED_APPS += $(APPDIR)/external/{crate_name}
    EXTRA_LIBS += \"{staticlib_path}\"
endif
        ",
    )
}

fn form_makefile(crate_name: &str, staticlib_path: &Utf8Path) -> String {
    let crate_name = crate_name.to_case(Case::UpperSnake);

    format!(
        "
include $(APPDIR)/Make.defs

PROGNAME  = $(CONFIG_CARGO_NUTTX_{crate_name}_PROGNAME)
PRIORITY  = $(CONFIG_CARGO_NUTTX_{crate_name}_PRIORITY)
STACKSIZE = $(CONFIG_CARGO_NUTTX_{crate_name}_STACKSIZE)
MODULE    = $(CONFIG_CARGO_NUTTX_{crate_name})

RUSTLIB := \"{staticlib_path}\"

context:: $(RUSTLIB)

$(RUSTLIB):

clean::

include $(APPDIR)/Application.mk
        "
    )
}

fn download_nuttx(version: &str, path: &Utf8Path) -> Result<(), Box<dyn Error>> {
    if path.join("nuttx").is_dir() && path.join("apps").is_dir() {
        return Ok(());
    }

    fs::create_dir_all(path)?;

    for url in [nuttx_url(version), nuttx_apps_url(version)] {
        let response = ureq::get(&url).call()?;
        let body = response.into_body();

        let gz_archive = GzDecoder::new(body.into_reader());
        let mut tar_archive = tar::Archive::new(gz_archive);

        tar_archive.unpack(path)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn build_nuttx(
    crate_name: &str,
    nuttx_path: NuttxLocation,
    board: &str,
    board_config: &str,
    config_fragment: &str,
    board_config_fragment: &str,
    build_target_name: &str,
    entrypoint_symbol: &str,
    release: bool,
    clean: bool,
    priority: u8,
    stack_size: u32,
    board_metadata: &BoardSpecificMetadata,
    make_args: impl IntoIterator<Item = OsString>,
) -> Result<Utf8PathBuf, Box<dyn Error>> {
    let path = match nuttx_path {
        NuttxLocation::Path { root } => root,
        NuttxLocation::Download { version } => {
            let path = Utf8PathBuf::try_from(
                cache_dir()
                    .ok_or("Could not locate cache dir for your os")?
                    .join(env!("CARGO_CRATE_NAME"))
                    .join("nuttx")
                    .join(crate_name)
                    .join(board),
            )?;

            download_nuttx(&version, &path)?;

            path
        }
    };

    let apps_path = path.join("apps");
    let nuttx_path = path.join("nuttx");

    if !nuttx_path.is_dir() || !apps_path.is_dir() {
        return Err("NuttX space is malformed".into());
    }

    let original_config = nuttx_path.join(format!(".config.{}", env!("CARGO_CRATE_NAME")));
    let config_path = nuttx_path.join(".config");

    let external_directory = apps_path.join("external");
    let shim_destination = external_directory.join(crate_name);

    // Set up NuttX so the header files are in place
    fs::create_dir_all(&shim_destination)?;

    let makefile = external_directory.join("Makefile");
    if !makefile.is_file() {
        fs::write(
            &makefile,
            "MENUDESC = \"External Apps\"
            include $(APPDIR)/Directory.mk",
        )?;
    }

    let make_defs = external_directory.join("Make.defs");
    if !make_defs.is_file() {
        fs::write(
            &make_defs,
            "include $(wildcard $(APPDIR)/external/*/Make.defs)",
        )
        .unwrap();
    }

    let kconfig = form_kconfig(crate_name, priority, stack_size);

    // Feed some impossible path as a dummy file
    let dummy_lib_path = Utf8PathBuf::from("/does/not/exist.a");
    let make_defs = form_make_defs(crate_name, &dummy_lib_path);
    let makefile = form_makefile(crate_name, &dummy_lib_path);

    fs::write(shim_destination.join("Kconfig"), kconfig)?;
    fs::write(shim_destination.join("Make.defs"), make_defs)?;
    fs::write(shim_destination.join("Makefile"), makefile)?;

    if clean {
        Command::new("make")
            .current_dir(&nuttx_path)
            .arg("distclean")
            .status()?;
    }

    // Configure for that board
    Command::new("./tools/configure.sh")
        .current_dir(&nuttx_path)
        .arg(format!("{board}:{board_config}"))
        .status()?;

    if clean || !original_config.is_file() {
        fs::copy(&config_path, &original_config)?;
    }

    fs::copy(&original_config, &config_path)?;

    // Inject the config fragments for the crate
    {
        let mut config_file = BufWriter::new(OpenOptions::new().append(true).open(&config_path)?);

        config_file.write_all("\n".as_bytes())?;
        config_file.write_all(config_fragment.as_bytes())?;
        config_file.write_all(board_config_fragment.as_bytes())?;

        let crate_name = crate_name.to_case(Case::UpperSnake);
        writeln!(&mut config_file, "CONFIG_CARGO_NUTTX_{crate_name}=y")?;
        writeln!(
            &mut config_file,
            "CONFIG_CARGO_NUTTX_{crate_name}_PRIORITY={priority}"
        )?;
        writeln!(
            &mut config_file,
            "CONFIG_CARGO_NUTTX_{crate_name}_STACK_SIZE={stack_size}"
        )?;
        writeln!(
            &mut config_file,
            "CONFIG_INIT_ENTRYPOINT=\"{}\"",
            entrypoint_symbol
        )?;
        writeln!(
            &mut config_file,
            "CONFIG_INIT_ENTRYNAME=\"{}\"",
            entrypoint_symbol
        )?;
    }

    Command::new("make")
        .current_dir(&nuttx_path)
        .arg("olddefconfig")
        .status()?;

    // Setup the headers
    Command::new("make")
        .current_dir(&nuttx_path)
        .arg("context")
        .status()?;

    // Do a build so we can extract the static library path
    let staticlib_path = build_cargo(
        crate_name,
        build_target_name,
        release,
        board_metadata,
        [
            nuttx_path.join("include"),
            nuttx_path.join("include").join("arch"),
        ],
    )?;

    // Setup NuttX *again* so our library is put into place correctly
    let make_defs = form_make_defs(crate_name, &staticlib_path);
    let makefile = form_makefile(crate_name, &staticlib_path);
    fs::write(shim_destination.join("Make.defs"), make_defs)?;
    fs::write(shim_destination.join("Makefile"), makefile)?;

    // Build nuttx
    Command::new("make")
        .current_dir(&nuttx_path)
        .args(make_args)
        .status()?;

    Ok(path)
}
