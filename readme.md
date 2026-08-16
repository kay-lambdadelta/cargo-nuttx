# cargo-nuttx

A Cargo subcommand for building Rust crates against [Apache NuttX](https://nuttx.apache.org/), without crafting your own scaffolding for doing so.

> **Not affiliated with or endorsed by the Apache Software Foundation.**

## Usage

Set in your project metadata sections a general metadata table:

```toml
[package.metadata.nuttx]
priority = 100
stack-size = 16384
# The NuttX base config, it will be added to the builtin board specific config you selected.
config = "shell/nuttallite/nuttx/config"
# Optional board specific config directory. The individual configs should have the same string as the board name, with no file extension.
board-config-directory = "shell/nuttallite/nuttx/config/board"
```

Then for each board you intend to support, add a table like this:

```toml
# The name should match the name of the board NuttX uses for `./tools/configure.sh`
[package.metadata.nuttx.boards.raspberrypi-pico-2]
# Denotes the rust target triplet to use for this board
target = "thumbv8m.main-nuttx-eabihf"
# Optional target CPU to use for this board
target-cpu = "cortex-m33"
# Optional optimization level to use for this board
opt-level = "s"
# The name of the firmware file this tool should pull into your target directory
firmware-file = "nuttx.u2f"
```

Invoking the `cargo nuttx build` command will build your project for the specified board and output in a folder called `nuttx` in your target directory.

Note that a nightly compiler _is_ required as this operates off `-Zbuild-std`

```bash
cargo +nightly nuttx --board raspberrypi-pico-2 --board-config nsh build -p fluxemu-shell-nuttallite --download --release --clean -- -j$(nproc)
```

Check `cargo nuttx --help` for more information.
