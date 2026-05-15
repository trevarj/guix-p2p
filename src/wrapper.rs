use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrapperProgram {
    GuixP2p,
    RealGuix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperExec {
    pub program: WrapperProgram,
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone)]
pub struct WrapperConfig {
    pub real_guix: OsString,
    pub guix_p2p: OsString,
    pub socket: PathBuf,
}

impl WrapperConfig {
    pub fn from_env() -> Self {
        WrapperConfig {
            real_guix: std::env::var_os("REAL_GUIX")
                .unwrap_or_else(|| OsString::from("/run/current-system/profile/bin/guix")),
            guix_p2p: std::env::var_os("GUIX_P2P_BIN")
                .unwrap_or_else(|| OsString::from("guix-p2p")),
            socket: std::env::var_os("GUIX_P2P_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(default_socket_path),
        }
    }

    pub fn program_path(&self, program: &WrapperProgram) -> &OsString {
        match program {
            WrapperProgram::GuixP2p => &self.guix_p2p,
            WrapperProgram::RealGuix => &self.real_guix,
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    cache_dir.join("guix-p2p/guix-p2p.sock")
}

pub fn route_invocation(args: &[OsString], socket_exists: bool, socket: &Path) -> WrapperExec {
    let is_substitute = args.first().is_some_and(|arg| arg == "substitute");
    if !is_substitute {
        return WrapperExec { program: WrapperProgram::RealGuix, args: args.to_vec() };
    }

    let substitute_args = &args[1..];
    let is_relay_mode =
        substitute_args.first().is_some_and(|arg| arg == "--query" || arg == "--substitute");

    if is_relay_mode && socket_exists {
        let mut relay_args = substitute_args.to_vec();
        relay_args.push(OsString::from("--socket"));
        relay_args.push(socket.as_os_str().to_os_string());
        WrapperExec { program: WrapperProgram::GuixP2p, args: relay_args }
    } else {
        WrapperExec { program: WrapperProgram::RealGuix, args: args.to_vec() }
    }
}

#[cfg(unix)]
pub fn socket_exists(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;

    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(not(unix))]
pub fn socket_exists(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn routes_query_to_p2p_when_socket_exists() {
        let socket = PathBuf::from("/tmp/guix-p2p.sock");
        let exec = route_invocation(&args(&["substitute", "--query"]), true, &socket);

        assert_eq!(exec.program, WrapperProgram::GuixP2p);
        assert_eq!(exec.args, args(&["--query", "--socket", "/tmp/guix-p2p.sock"]));
    }

    #[test]
    fn routes_substitute_to_p2p_when_socket_exists() {
        let socket = PathBuf::from("/tmp/guix-p2p.sock");
        let exec = route_invocation(
            &args(&["substitute", "--substitute", "/gnu/store/example", "/tmp/out"]),
            true,
            &socket,
        );

        assert_eq!(exec.program, WrapperProgram::GuixP2p);
        assert_eq!(
            exec.args,
            args(&[
                "--substitute",
                "/gnu/store/example",
                "/tmp/out",
                "--socket",
                "/tmp/guix-p2p.sock"
            ])
        );
    }

    #[test]
    fn falls_back_to_real_guix_when_socket_is_missing() {
        let socket = PathBuf::from("/tmp/missing.sock");
        let original = args(&["substitute", "--query"]);
        let exec = route_invocation(&original, false, &socket);

        assert_eq!(exec.program, WrapperProgram::RealGuix);
        assert_eq!(exec.args, original);
    }

    #[test]
    fn passes_unknown_substitute_invocation_to_real_guix() {
        let socket = PathBuf::from("/tmp/guix-p2p.sock");
        let original = args(&["substitute", "--help"]);
        let exec = route_invocation(&original, true, &socket);

        assert_eq!(exec.program, WrapperProgram::RealGuix);
        assert_eq!(exec.args, original);
    }

    #[test]
    fn passes_regular_guix_command_to_real_guix() {
        let socket = PathBuf::from("/tmp/guix-p2p.sock");
        let original = args(&["build", "hello"]);
        let exec = route_invocation(&original, true, &socket);

        assert_eq!(exec.program, WrapperProgram::RealGuix);
        assert_eq!(exec.args, original);
    }
}
