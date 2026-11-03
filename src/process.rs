use std::{error::Error, process::Command};

pub fn run(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let program = command.get_program().to_string_lossy().into_owned();
    let exit_status = command
        .status()
        .map_err(|error| format!("Could not run `{program}`: {error}"))?;

    if !exit_status.success() {
        return Err(format!("`{program}` failed ({exit_status})").into());
    }

    Ok(())
}
