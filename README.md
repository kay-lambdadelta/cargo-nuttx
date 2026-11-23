# cargo-nuttx

A Cargo subcommand for building Rust crates against [Apache NuttX](https://nuttx.apache.org/), without crafting your own scaffolding for doing so.

> [!IMPORTANT]
> **Not affiliated with or endorsed by the Apache Software Foundation.**

## Usage

Your crate needs to be built as a static library:

```toml
[lib]
crate-type = ["staticlib"]
```

Set in your project metadata sections a general metadata table. Paths are relative to the `Cargo.toml` they are in.

```toml
[package.metadata.nuttx]
priority = 100
stack-size = 16384
# Optional directory holding NuttX specific files, see below (default: `nuttx`)
directory = "nuttx"
# Optional name of the program inside NuttX (default: the crate name in snake case)
program-name = "example"
# Optional entry point symbol (default: <crate_name>_main)
entrypoint-symbol = "example_main"
# Optional NuttX release to download (default: the latest version this tool was tested with)
nuttx-version = "13.1.0"
```

Then for each board you intend to support, add a table like this:

```toml
# The name should match the name of the board NuttX uses for `./tools/configure.sh`
[package.metadata.nuttx.boards.raspberrypi-pico-2]
# Denotes the rust target triplet to use for this board
target = "thumbv8m.main-nuttx-eabihf"
# The name of the firmware file this tool should pull into your target directory
firmware-file = "nuttx.uf2"
# Optional base board config to use (default: nsh)
board-config = "nsh"
# Optional target CPU to use for this board
target-cpu = "cortex-m33"
# Optional optimization level to use for this board
opt-level = "s"
# Optional command `cargo nuttx run` uses to flash the firmware, the firmware path is appended
runner = ["picotool", "load", "-x"]
```

By default, the tool assumes the entry point for the program is <crate_name>_main, of which the signature should look like this:

```rust
use std::ffi::{c_char, c_int};

#[unsafe(no_mangle)]
unsafe extern "C" fn example_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    println!("Hello, world!");

    0
}
```

The tool selects `CONFIG_LIBM_OPENLIBM` by default, as it is the only `libm` that works with Rust's std so far.

## NuttX directory

Everything beyond the metadata lives in the NuttX directory:

| Path                          | Purpose                                              |
| ----------------------------- | ---------------------------------------------------- |
| `config`                      | Config fragment added to every board's config        |
| `patches/nuttx/*.patch`       | Patches applied to the NuttX tree                    |
| `patches/apps/*.patch`        | Patches applied to the NuttX apps tree               |
| `boards/<board>/config`       | Config fragment added to this board's config only    |
| `boards/<board>/linker.ld`    | Linker script replacing the board's own              |
| `boards/<board>/patches/`     | Patches for this board only                          |
| `boards/<board>/custom/`      | Custom board, used in place of NuttX's builtin board |
| `boards/<board>/custom/rust/` | Optional board crate                                 |

### Patches

Patches are applied in filename order, global patches and then board patches.
Changing the patch set extracts a clean tree before applying them again.

Patches are only applied to the NuttX tree this tool downloads itself.

### Custom boards

A `custom` directory is a regular NuttX board directory, used through `CONFIG_ARCH_BOARD_CUSTOM_DIR`.
It needs at least `scripts/Make.defs` and a `configs/<board-config>/defconfig`, where the `board-config` of the board selects which one is used.

If `custom/rust` contains a valid crate, it is built as a board crate for the same target with `-Zbuild-std=core`, and linked into the final binary.

### Include directories

Both your crate and the board crate are built with `NUTTX_INCLUDE_DIR` set to a `:` separated list of the configured NuttX include directories.

## Commands

- `cargo nuttx build` builds your crate and NuttX, and outputs the firmware in `target/nuttx/<board>/<profile>/`
- `cargo nuttx run` does the same, then runs the board's `runner` on the firmware
- `cargo nuttx check` and `cargo nuttx clippy` check your crate against the configured NuttX headers

Note that a nightly compiler _is_ required as this operates off `-Zbuild-std`

```bash
cargo +nightly nuttx build --board raspberrypi-pico-2 --release
```

`--board` can be left out if your crate only lists one board. NuttX is downloaded into your cache directory unless you point the tool at your own checkout with `--nuttx-path` (a directory containing `nuttx` and `apps`).

Unknown arguments are passed to cargo, and anything after `--` goes to rustc or clippy, just like with cargo itself:

```bash
cargo +nightly nuttx clippy --no-default-features --features egui -- -D warnings
```

Arguments for NuttX's `make` go through `--make-arg`, e.g. `--make-arg V=1`.

Check `cargo nuttx --help` for more information.
