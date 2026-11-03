# cargo-nuttx

A Cargo subcommand for building Rust crates against [Apache NuttX](https://nuttx.apache.org/), without crafting your own scaffolding for doing so.

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
# Optional NuttX config fragment, it will be added to the builtin board specific config you selected.
config = "nuttx/config"
# Optional board specific config directory. The individual configs should have the same string as the board name, with no file extension.
board-config-directory = "nuttx/board"
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
#[unsafe(no_mangle)]
unsafe extern "C" fn example_main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("Hello, world!");

    0
}
```

The tool selects `CONFIG_LIBM_OPENLIBM` by default, as it is the only libm that works with Rust's std so far.

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
