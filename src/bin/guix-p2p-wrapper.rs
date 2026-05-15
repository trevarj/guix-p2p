use std::{os::unix::process::CommandExt, process::Command};

use guix_p2p::wrapper::{WrapperConfig, route_invocation, socket_exists};

fn main() -> anyhow::Result<()> {
    let config = WrapperConfig::from_env();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let target = route_invocation(&args, socket_exists(&config.socket), &config.socket);

    let mut command = Command::new(config.program_path(&target.program));
    command.args(target.args);
    Err(command.exec()).map_err(|error| anyhow::anyhow!("failed to exec wrapper target: {error}"))
}
