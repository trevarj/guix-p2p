use std::{
    ffi::OsString,
    net::{Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
};

use libp2p::{Multiaddr, multiaddr::Protocol};
use serde::Serialize;

use crate::{config::Config, peer_store};

/// Severity for a local rollout-readiness diagnostic item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticSeverity {
    /// The check passed or is informational.
    Ok,
    /// The node can run, but testers may see degraded connectivity or UX.
    Warning,
    /// A required local prerequisite is missing.
    Error,
}

/// One local diagnostic finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticCheck {
    pub id: &'static str,
    pub severity: DiagnosticSeverity,
    pub summary: String,
    pub detail: String,
}

/// Machine-readable report emitted by `guix-p2p --doctor --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticReport {
    pub peer_id: String,
    pub connectivity: ConnectivitySummary,
    pub checks: Vec<DiagnosticCheck>,
    pub has_errors: bool,
    pub has_warnings: bool,
}

/// Shareable peer information for onboarding testers and bootstrap nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootstrapBundle {
    pub peer_id: String,
    pub connectivity: ConnectivitySummary,
    pub shareable_addresses: Vec<String>,
    pub bootstrap_peers: Vec<String>,
    pub dashboard_url: Option<String>,
    pub has_errors: bool,
    pub has_warnings: bool,
    pub config_snippet: String,
}

/// Compact connectivity status shown by the dashboard and `--doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectivitySummary {
    pub state: &'static str,
    pub detail: String,
    pub has_bootstrap_peers: bool,
    pub has_shareable_addresses: bool,
    pub has_private_external_addresses: bool,
    pub shareable_addresses: Vec<String>,
}

/// Build a local diagnostic report from the loaded configuration.
pub fn run_config_diagnostics(config: &Config, peer_id: &str) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    let summary = connectivity_summary(config, peer_id);
    let listen_addr_error = multiaddr_parse_error(&config.listen_addr);
    let invalid_bootstrap_peers = invalid_multiaddrs(&config.bootstrap_peers);
    let bootstrap_without_peer_id = bootstrap_peers_without_peer_id(&config.bootstrap_peers);
    let invalid_external_addresses = invalid_multiaddrs(&config.external_addresses);

    checks.push(DiagnosticCheck {
        id: "identity",
        severity: DiagnosticSeverity::Ok,
        summary: "identity loaded".to_string(),
        detail: format!("peer id: {peer_id}"),
    });

    checks.push(DiagnosticCheck {
        id: "listen-address",
        severity: if listen_addr_error.is_none() {
            DiagnosticSeverity::Ok
        } else {
            DiagnosticSeverity::Error
        },
        summary: if listen_addr_error.is_none() {
            "listen address is valid".to_string()
        } else {
            "listen address is invalid".to_string()
        },
        detail: listen_addr_error
            .map(|err| format!("{}: {err}", config.listen_addr))
            .unwrap_or_else(|| config.listen_addr.clone()),
    });

    checks.push(DiagnosticCheck {
        id: "bootstrap-peers",
        severity: if !invalid_bootstrap_peers.is_empty() {
            DiagnosticSeverity::Error
        } else if !bootstrap_without_peer_id.is_empty() {
            DiagnosticSeverity::Warning
        } else if summary.has_bootstrap_peers {
            DiagnosticSeverity::Ok
        } else {
            DiagnosticSeverity::Warning
        },
        summary: if !invalid_bootstrap_peers.is_empty() {
            "invalid bootstrap peer address".to_string()
        } else if !bootstrap_without_peer_id.is_empty() {
            "bootstrap peer missing /p2p peer id".to_string()
        } else if summary.has_bootstrap_peers {
            "bootstrap peers configured".to_string()
        } else {
            "no bootstrap peers configured".to_string()
        },
        detail: if !invalid_bootstrap_peers.is_empty() {
            invalid_bootstrap_peers.join(", ")
        } else if !bootstrap_without_peer_id.is_empty() {
            format!("{} should end in /p2p/<peer-id>", bootstrap_without_peer_id.join(", "))
        } else if summary.has_bootstrap_peers {
            format!("{} bootstrap peer(s)", config.bootstrap_peers.len())
        } else {
            "configure bootstrap_peers or rely on LAN mDNS only".to_string()
        },
    });

    if !config.external_addresses.is_empty() {
        checks.push(DiagnosticCheck {
            id: "external-addresses",
            severity: if invalid_external_addresses.is_empty() {
                DiagnosticSeverity::Ok
            } else {
                DiagnosticSeverity::Error
            },
            summary: if invalid_external_addresses.is_empty() {
                "external addresses are valid".to_string()
            } else {
                "invalid external address".to_string()
            },
            detail: if invalid_external_addresses.is_empty() {
                config.external_addresses.join(", ")
            } else {
                invalid_external_addresses.join(", ")
            },
        });
    }

    checks.push(DiagnosticCheck {
        id: "shareable-address",
        severity: if summary.has_shareable_addresses {
            DiagnosticSeverity::Ok
        } else {
            DiagnosticSeverity::Warning
        },
        summary: if summary.has_shareable_addresses {
            "shareable peer address available".to_string()
        } else {
            "no shareable peer address".to_string()
        },
        detail: if summary.has_shareable_addresses {
            summary.shareable_addresses.join(", ")
        } else {
            "set external_addresses to a public DNS/IP multiaddr when peers must dial this node"
                .to_string()
        },
    });

    if summary.has_private_external_addresses {
        checks.push(DiagnosticCheck {
            id: "nat-address",
            severity: DiagnosticSeverity::Warning,
            summary: "external address looks private or loopback".to_string(),
            detail: "this is fine for LAN tests, but remote testers usually need port forwarding, \
                     IPv6, or a public DNS/IP address"
                .to_string(),
        });
    }

    checks.push(path_check(
        "cache-dir",
        &config.cache_dir,
        config.cache_dir.exists(),
        "cache directory exists",
        "cache directory does not exist yet",
        "it will be created when the daemon writes identity/cache data",
        DiagnosticSeverity::Warning,
    ));

    checks.push(path_check(
        "acl",
        &config.acl_path,
        config.acl_path.exists(),
        "Guix substitute ACL exists",
        "Guix substitute ACL missing",
        "narinfo signature verification needs the Guix ACL path",
        DiagnosticSeverity::Error,
    ));

    checks.push(daemon_socket_check(&config.socket_path));

    checks.push(guix_integration_check());

    checks.push(DiagnosticCheck {
        id: "substitute-urls",
        severity: if config.substitute_urls.is_empty() {
            DiagnosticSeverity::Error
        } else {
            DiagnosticSeverity::Ok
        },
        summary: if config.substitute_urls.is_empty() {
            "no substitute URLs configured".to_string()
        } else {
            "substitute URLs configured".to_string()
        },
        detail: config.substitute_urls.join(", "),
    });

    checks
}

/// Build a complete local diagnostic report.
pub fn diagnostic_report(config: &Config, peer_id: &str) -> DiagnosticReport {
    let checks = run_config_diagnostics(config, peer_id);
    DiagnosticReport {
        peer_id: peer_id.to_string(),
        connectivity: connectivity_summary(config, peer_id),
        has_errors: has_diagnostic_errors(&checks),
        has_warnings: checks.iter().any(|check| check.severity == DiagnosticSeverity::Warning),
        checks,
    }
}

/// Build the shareable bootstrap bundle for this node.
pub fn bootstrap_bundle(config: &Config, peer_id: &str) -> BootstrapBundle {
    let report = diagnostic_report(config, peer_id);
    let shareable_addresses = report.connectivity.shareable_addresses.clone();
    BootstrapBundle {
        peer_id: peer_id.to_string(),
        connectivity: report.connectivity,
        bootstrap_peers: shareable_addresses.clone(),
        shareable_addresses: shareable_addresses.clone(),
        dashboard_url: dashboard_url(config),
        has_errors: report.has_errors,
        has_warnings: report.has_warnings,
        config_snippet: bootstrap_config_snippet(&shareable_addresses),
    }
}

/// Return true when any diagnostic check is an error.
pub fn has_diagnostic_errors(checks: &[DiagnosticCheck]) -> bool {
    checks.iter().any(|check| check.severity == DiagnosticSeverity::Error)
}

/// Summarize whether this node has enough static configuration to be reachable.
pub fn connectivity_summary(config: &Config, peer_id: &str) -> ConnectivitySummary {
    connectivity_summary_from_parts(&config.bootstrap_peers, &config.external_addresses, peer_id)
}

/// Summarize connectivity from the parts exposed by the dashboard state.
pub fn connectivity_summary_from_parts(
    bootstrap_peers: &[String],
    external_addresses: &[String],
    peer_id: &str,
) -> ConnectivitySummary {
    let shareable_addresses = peer_store::shareable_addresses(external_addresses, peer_id);
    let has_private_external_addresses =
        external_addresses.iter().any(|addr| multiaddr_has_private_ip(addr));
    let has_bootstrap_peers = !bootstrap_peers.is_empty();
    let has_shareable_addresses = !shareable_addresses.is_empty();

    let (state, detail) = if has_shareable_addresses && has_bootstrap_peers {
        ("shareable", "bootstrap peers and shareable addresses are configured")
    } else if has_shareable_addresses {
        ("shareable-no-bootstrap", "shareable address configured, but no bootstrap peers")
    } else if has_bootstrap_peers {
        ("client-only", "bootstrap peers configured; no shareable address for inbound peers")
    } else {
        ("local-only", "no bootstrap peers or shareable address; LAN mDNS may still work")
    };

    ConnectivitySummary {
        state,
        detail: detail.to_string(),
        has_bootstrap_peers,
        has_shareable_addresses,
        has_private_external_addresses,
        shareable_addresses,
    }
}

fn path_check(
    id: &'static str,
    path: &Path,
    exists: bool,
    ok_summary: &str,
    missing_summary: &str,
    missing_detail: &str,
    missing_severity: DiagnosticSeverity,
) -> DiagnosticCheck {
    DiagnosticCheck {
        id,
        severity: if exists { DiagnosticSeverity::Ok } else { missing_severity },
        summary: if exists { ok_summary } else { missing_summary }.to_string(),
        detail: if exists { path.display().to_string() } else { missing_detail.to_string() },
    }
}

#[cfg(unix)]
fn daemon_socket_check(socket_path: &str) -> DiagnosticCheck {
    use std::os::unix::{fs::FileTypeExt, net::UnixStream};

    let path = Path::new(socket_path);
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return DiagnosticCheck {
                id: "daemon",
                severity: DiagnosticSeverity::Error,
                summary: "guix-p2p daemon is not running".to_string(),
                detail: format!(
                    "daemon socket {} is not present: {error}; start guix-p2p --daemon with the \
                     configured socket path",
                    path.display()
                ),
            };
        },
    };

    if !metadata.file_type().is_socket() {
        return DiagnosticCheck {
            id: "daemon",
            severity: DiagnosticSeverity::Error,
            summary: "daemon socket path is not a Unix socket".to_string(),
            detail: format!(
                "{} exists but is not a Unix socket; remove it and restart guix-p2p --daemon",
                path.display()
            ),
        };
    }

    match UnixStream::connect(path) {
        Ok(_) => DiagnosticCheck {
            id: "daemon",
            severity: DiagnosticSeverity::Ok,
            summary: "guix-p2p daemon accepts relay connections".to_string(),
            detail: path.display().to_string(),
        },
        Err(error) => DiagnosticCheck {
            id: "daemon",
            severity: DiagnosticSeverity::Error,
            summary: "guix-p2p daemon is not accepting relay connections".to_string(),
            detail: format!(
                "failed to connect to {}: {error}; the socket may be stale or the daemon may be \
                 stopped",
                path.display()
            ),
        },
    }
}

#[cfg(not(unix))]
fn daemon_socket_check(socket_path: &str) -> DiagnosticCheck {
    DiagnosticCheck {
        id: "daemon",
        severity: DiagnosticSeverity::Error,
        summary: "guix-p2p daemon check requires Unix sockets".to_string(),
        detail: format!("{socket_path} cannot be checked on this platform"),
    }
}

fn multiaddr_has_private_ip(value: &str) -> bool {
    let Ok(addr) = value.parse::<Multiaddr>() else {
        return false;
    };
    addr.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => ipv4_is_private_or_loopback(ip),
        Protocol::Ip6(ip) => ipv6_is_private_or_loopback(ip),
        _ => false,
    })
}

fn multiaddr_parse_error(value: &str) -> Option<String> {
    value.parse::<Multiaddr>().err().map(|err| err.to_string())
}

fn invalid_multiaddrs(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| multiaddr_parse_error(value).map(|err| format!("{value}: {err}")))
        .collect()
}

fn bootstrap_peers_without_peer_id(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| {
            let addr = value.parse::<Multiaddr>().ok()?;
            if matches!(addr.iter().last(), Some(Protocol::P2p(_))) {
                None
            } else {
                Some(value.clone())
            }
        })
        .collect()
}

fn dashboard_url(config: &Config) -> Option<String> {
    if !config.dashboard_enabled {
        return None;
    }
    let host = match config.dashboard_bind.as_str() {
        "0.0.0.0" | "::" => "127.0.0.1",
        bind => bind,
    };
    Some(format!("http://{host}:{}", config.dashboard_port))
}

fn bootstrap_config_snippet(bootstrap_peers: &[String]) -> String {
    if bootstrap_peers.is_empty() {
        "# Set external_addresses first, then run guix-p2p --share-info again.".to_string()
    } else {
        format!("bootstrap_peers = \"{}\"", bootstrap_peers.join(","))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuixIntegrationKind {
    Extension,
    Wrapper,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuixIntegrationEvidence {
    kind: GuixIntegrationKind,
    detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GuixDaemonEnvProbe {
    found_daemons: usize,
    readable_envs: usize,
    unreadable_envs: usize,
    evidence: Vec<GuixIntegrationEvidence>,
}

fn guix_integration_check() -> DiagnosticCheck {
    let daemon_probe = inspect_guix_daemon_environments();
    if let Some(evidence) = daemon_probe.evidence.first() {
        return DiagnosticCheck {
            id: "guix-integration",
            severity: DiagnosticSeverity::Ok,
            summary: match evidence.kind {
                GuixIntegrationKind::Extension => "Guix substitute extension enabled".to_string(),
                GuixIntegrationKind::Wrapper => "Guix wrapper integration enabled".to_string(),
            },
            detail: format!(
                "{}; inspected {} running guix-daemon environment(s)",
                evidence.detail, daemon_probe.readable_envs
            ),
        };
    }

    let current_env = guix_integration_evidence_from_env(std::env::vars_os());
    let current_env_detail =
        current_env.first().map(|evidence| format!(" Current process: {}.", evidence.detail));

    let detail = if daemon_probe.found_daemons == 0 {
        format!(
            "no running guix-daemon process was found; enable \
             guix-p2p-enable-guix-daemon-extension or the legacy wrapper in the guix-service \
             environment.{}",
            current_env_detail.unwrap_or_default()
        )
    } else if daemon_probe.readable_envs == 0 {
        format!(
            "found {} guix-daemon process(es), but their environments were not readable; run \
             doctor with enough permissions or verify that GUIX_EXTENSIONS_PATH, GUIX_P2P_BIN, \
             and GUIX_P2P_SOCKET are set in the guix-service environment.{}",
            daemon_probe.found_daemons,
            current_env_detail.unwrap_or_default()
        )
    } else {
        format!(
            "inspected {} guix-daemon environment(s); none had the guix-p2p substitute extension \
             or wrapper enabled. Enable guix-p2p-enable-guix-daemon-extension in the guix-service \
             configuration.{}",
            daemon_probe.readable_envs,
            current_env_detail.unwrap_or_default()
        )
    };

    DiagnosticCheck {
        id: "guix-integration",
        severity: DiagnosticSeverity::Error,
        summary: "Guix substitute integration not confirmed".to_string(),
        detail,
    }
}

fn guix_integration_evidence_from_env<I>(vars: I) -> Vec<GuixIntegrationEvidence>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let vars: Vec<(String, String)> = vars
        .into_iter()
        .map(|(key, value)| {
            (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())
        })
        .collect();
    let value =
        |name: &str| vars.iter().find_map(|(key, value)| (key == name).then_some(value.as_str()));

    let mut evidence = Vec::new();

    if let Some(guix) = value("GUIX")
        && guix.contains("guix-p2p-wrapper")
    {
        evidence.push(GuixIntegrationEvidence {
            kind: GuixIntegrationKind::Wrapper,
            detail: format!("GUIX points to {guix}"),
        });
    }

    if let Some(extensions_path) = value("GUIX_EXTENSIONS_PATH") {
        let has_extension_path = split_env_paths(extensions_path)
            .iter()
            .any(|path| path_contains_guix_p2p_extension(path));
        let has_helper_vars = value("GUIX_P2P_BIN").is_some() && value("GUIX_P2P_SOCKET").is_some();
        if has_extension_path || has_helper_vars {
            let detail = if has_extension_path {
                format!("GUIX_EXTENSIONS_PATH includes {extensions_path}")
            } else {
                "GUIX_EXTENSIONS_PATH plus GUIX_P2P_BIN and GUIX_P2P_SOCKET are set".to_string()
            };
            evidence.push(GuixIntegrationEvidence { kind: GuixIntegrationKind::Extension, detail });
        }
    }

    evidence
}

fn split_env_paths(value: &str) -> Vec<PathBuf> {
    value.split(':').filter(|path| !path.is_empty()).map(PathBuf::from).collect()
}

fn path_contains_guix_p2p_extension(path: &Path) -> bool {
    path.to_string_lossy().contains("guix-p2p")
        || path.join("substitute.scm").is_file()
        || path.join("guix/extensions/substitute.scm").is_file()
}

fn inspect_guix_daemon_environments() -> GuixDaemonEnvProbe {
    let mut probe = GuixDaemonEnvProbe::default();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return probe;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        if !file_name.to_string_lossy().bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }

        let process_dir = entry.path();
        if !process_looks_like_guix_daemon(&process_dir) {
            continue;
        }

        probe.found_daemons += 1;
        match read_proc_environ(&process_dir) {
            Ok(vars) => {
                probe.readable_envs += 1;
                probe.evidence.extend(guix_integration_evidence_from_env(vars));
            },
            Err(_) => {
                probe.unreadable_envs += 1;
            },
        }
    }

    probe
}

fn process_looks_like_guix_daemon(process_dir: &Path) -> bool {
    let comm = std::fs::read_to_string(process_dir.join("comm")).unwrap_or_default();
    if comm.trim() == "guix-daemon" {
        return true;
    }

    let cmdline = std::fs::read(process_dir.join("cmdline")).unwrap_or_default();
    cmdline.split(|byte| *byte == 0).any(|arg| String::from_utf8_lossy(arg).contains("guix-daemon"))
}

fn read_proc_environ(process_dir: &Path) -> std::io::Result<Vec<(OsString, OsString)>> {
    let environ = std::fs::read(process_dir.join("environ"))?;
    Ok(environ
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let split = entry.iter().position(|byte| *byte == b'=')?;
            let (key, value_with_separator) = entry.split_at(split);
            let value = &value_with_separator[1..];
            Some((
                OsString::from(String::from_utf8_lossy(key).into_owned()),
                OsString::from(String::from_utf8_lossy(value).into_owned()),
            ))
        })
        .collect())
}

fn ipv4_is_private_or_loopback(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local()
}

fn ipv6_is_private_or_loopback(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
}

/// Format diagnostics as a terminal-friendly report.
pub fn format_diagnostics(checks: &[DiagnosticCheck]) -> String {
    format_diagnostics_with_color(checks, false)
}

/// Format diagnostics as a terminal-friendly report, optionally using ANSI color.
pub fn format_diagnostics_with_color(checks: &[DiagnosticCheck], color: bool) -> String {
    let mut out = String::new();
    let errors = checks.iter().filter(|check| check.severity == DiagnosticSeverity::Error).count();
    let warnings =
        checks.iter().filter(|check| check.severity == DiagnosticSeverity::Warning).count();
    let ok = checks.iter().filter(|check| check.severity == DiagnosticSeverity::Ok).count();
    let status = if errors > 0 {
        paint("not ready", AnsiColor::Red, color)
    } else if warnings > 0 {
        paint("ready with warnings", AnsiColor::Yellow, color)
    } else {
        paint("ready", AnsiColor::Green, color)
    };

    out.push_str(&format!("{}\n", paint("guix-p2p doctor", AnsiColor::Bold, color)));
    out.push_str(&format!("status: {status}  {ok} ok, {warnings} warning(s), {errors} error(s)\n"));

    let sections: &[(&str, &[&str])] = &[
        ("Guix substitute path", &["identity", "guix-integration", "substitute-urls", "acl"]),
        (
            "Connectivity",
            &[
                "listen-address",
                "bootstrap-peers",
                "external-addresses",
                "shareable-address",
                "nat-address",
            ],
        ),
        ("Runtime", &["cache-dir", "daemon"]),
    ];

    for (title, ids) in sections {
        let section_checks: Vec<&DiagnosticCheck> =
            ids.iter().filter_map(|id| checks.iter().find(|check| check.id == *id)).collect();
        if section_checks.is_empty() {
            continue;
        }

        out.push('\n');
        out.push_str(&format!("{}\n", paint(title, AnsiColor::Bold, color)));
        for check in section_checks {
            out.push_str(&format_diagnostic_check(check, color));
        }
    }

    let known_ids: Vec<&str> = sections.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
    let other_checks: Vec<&DiagnosticCheck> =
        checks.iter().filter(|check| !known_ids.contains(&check.id)).collect();
    if !other_checks.is_empty() {
        out.push('\n');
        out.push_str(&format!("{}\n", paint("Other", AnsiColor::Bold, color)));
        for check in other_checks {
            out.push_str(&format_diagnostic_check(check, color));
        }
    }
    out
}

fn format_diagnostic_check(check: &DiagnosticCheck, color: bool) -> String {
    let marker_text = match check.severity {
        DiagnosticSeverity::Ok => "ok",
        DiagnosticSeverity::Warning => "warn",
        DiagnosticSeverity::Error => "error",
    };
    let marker_color = match check.severity {
        DiagnosticSeverity::Ok => AnsiColor::Green,
        DiagnosticSeverity::Warning => AnsiColor::Yellow,
        DiagnosticSeverity::Error => AnsiColor::Red,
    };
    let marker = paint(&format!("{marker_text:5}"), marker_color, color);
    let id = paint(&format!("{:<20}", check.id), AnsiColor::Cyan, color);
    format!("  {marker} {id} {}\n        {}\n", check.summary, check.detail)
}

#[derive(Debug, Clone, Copy)]
enum AnsiColor {
    Bold,
    Cyan,
    Green,
    Red,
    Yellow,
}

fn paint(value: &str, color: AnsiColor, enabled: bool) -> String {
    if !enabled {
        return value.to_string();
    }
    let code = match color {
        AnsiColor::Bold => "1",
        AnsiColor::Cyan => "36",
        AnsiColor::Green => "32",
        AnsiColor::Red => "31",
        AnsiColor::Yellow => "33",
    };
    format!("\x1b[{code}m{value}\x1b[0m")
}

/// Format the bootstrap bundle as a terminal-friendly report.
pub fn format_bootstrap_bundle(bundle: &BootstrapBundle) -> String {
    let mut out = String::new();
    out.push_str(&format!("peer id: {}\n", bundle.peer_id));
    out.push_str(&format!(
        "network: {} ({})\n",
        bundle.connectivity.state, bundle.connectivity.detail
    ));
    if let Some(url) = &bundle.dashboard_url {
        out.push_str(&format!("dashboard: {url}\n"));
    }
    out.push_str("shareable addresses:\n");
    if bundle.shareable_addresses.is_empty() {
        out.push_str("  none\n");
    } else {
        for address in &bundle.shareable_addresses {
            out.push_str(&format!("  {address}\n"));
        }
    }
    out.push_str("tester config:\n");
    out.push_str(&format!("  {}\n", bundle.config_snippet));
    if bundle.has_errors {
        out.push_str("status: diagnostics have errors; run guix-p2p --doctor\n");
    } else if bundle.has_warnings {
        out.push_str("status: diagnostics have warnings; run guix-p2p --doctor\n");
    } else {
        out.push_str("status: ready\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_addresses(addresses: Vec<String>, bootstrap: Vec<String>) -> Config {
        let mut config =
            Config::load(None, None, None, Some("/tmp/guix-p2p-diag".into()), None, None);
        config.external_addresses = addresses;
        config.bootstrap_peers = bootstrap;
        config
    }

    #[test]
    fn connectivity_summary_detects_shareable_address() {
        let peer = libp2p::PeerId::random().to_string();
        let config = config_with_addresses(
            vec!["/dns4/node.example.org/udp/6881/quic-v1".to_string()],
            vec!["/dns4/bootstrap.example.org/udp/6881/quic-v1/p2p/12D3KooWQp4D6Lwq".to_string()],
        );

        let summary = connectivity_summary(&config, &peer);

        assert_eq!(summary.state, "shareable");
        assert!(summary.has_bootstrap_peers);
        assert!(summary.has_shareable_addresses);
        assert_eq!(
            summary.shareable_addresses,
            vec![format!("/dns4/node.example.org/udp/6881/quic-v1/p2p/{peer}")]
        );
    }

    #[test]
    fn connectivity_summary_warns_for_local_only_node() {
        let config = config_with_addresses(vec![], vec![]);

        let summary = connectivity_summary(&config, "not-a-peer");

        assert_eq!(summary.state, "local-only");
        assert!(!summary.has_bootstrap_peers);
        assert!(!summary.has_shareable_addresses);
    }

    #[test]
    fn diagnostics_include_private_external_address_warning() {
        let peer = libp2p::PeerId::random().to_string();
        let config =
            config_with_addresses(vec!["/ip4/192.168.1.20/udp/6881/quic-v1".to_string()], vec![]);

        let checks = run_config_diagnostics(&config, &peer);

        assert!(checks.iter().any(|check| check.id == "nat-address"));
    }

    #[test]
    fn diagnostics_reject_invalid_listen_address() {
        let peer = libp2p::PeerId::random().to_string();
        let mut config = config_with_addresses(vec![], vec![]);
        config.listen_addr = "not-a-multiaddr".to_string();

        let checks = run_config_diagnostics(&config, &peer);

        let check = checks.iter().find(|check| check.id == "listen-address").unwrap();
        assert_eq!(check.severity, DiagnosticSeverity::Error);
        assert!(check.summary.contains("invalid"));
    }

    #[test]
    fn diagnostics_reject_invalid_bootstrap_peer() {
        let peer = libp2p::PeerId::random().to_string();
        let config = config_with_addresses(
            vec![],
            vec!["/dns4/bootstrap.example.org/not-a-transport".to_string()],
        );

        let checks = run_config_diagnostics(&config, &peer);

        let check = checks.iter().find(|check| check.id == "bootstrap-peers").unwrap();
        assert_eq!(check.severity, DiagnosticSeverity::Error);
        assert!(check.summary.contains("invalid"));
    }

    #[test]
    fn diagnostics_warn_when_bootstrap_peer_lacks_peer_id() {
        let peer = libp2p::PeerId::random().to_string();
        let config = config_with_addresses(
            vec![],
            vec!["/dns4/bootstrap.example.org/udp/6881/quic-v1".to_string()],
        );

        let checks = run_config_diagnostics(&config, &peer);

        let check = checks.iter().find(|check| check.id == "bootstrap-peers").unwrap();
        assert_eq!(check.severity, DiagnosticSeverity::Warning);
        assert!(check.detail.contains("/p2p/<peer-id>"));
    }

    #[test]
    fn diagnostics_reject_invalid_external_address() {
        let peer = libp2p::PeerId::random().to_string();
        let config = config_with_addresses(vec!["not-a-multiaddr".to_string()], vec![]);

        let checks = run_config_diagnostics(&config, &peer);

        let check = checks.iter().find(|check| check.id == "external-addresses").unwrap();
        assert_eq!(check.severity, DiagnosticSeverity::Error);
        assert!(check.summary.contains("invalid"));
    }

    #[test]
    fn guix_integration_detects_extension_environment() {
        let evidence = guix_integration_evidence_from_env([
            (
                "GUIX_EXTENSIONS_PATH".into(),
                "/gnu/store/hash-guix-p2p/share/guix/extensions".into(),
            ),
            ("GUIX_P2P_BIN".into(), "/run/current-system/profile/bin/guix-p2p".into()),
            ("GUIX_P2P_SOCKET".into(), "/var/cache/guix-p2p/guix-p2p.sock".into()),
        ]);

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].kind, GuixIntegrationKind::Extension);
    }

    #[test]
    fn guix_integration_detects_wrapper_environment() {
        let evidence = guix_integration_evidence_from_env([
            ("GUIX".into(), "/run/current-system/profile/bin/guix-p2p-wrapper".into()),
            ("REAL_GUIX".into(), "/run/current-system/profile/bin/guix".into()),
        ]);

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].kind, GuixIntegrationKind::Wrapper);
    }

    #[test]
    fn guix_integration_ignores_plain_guix_environment() {
        let evidence = guix_integration_evidence_from_env([
            ("GUIX".into(), "/run/current-system/profile/bin/guix".into()),
            (
                "GUIX_EXTENSIONS_PATH".into(),
                "/run/current-system/profile/share/guix/extensions".into(),
            ),
        ]);

        assert!(evidence.is_empty());
    }

    #[test]
    fn guix_integration_missing_is_error() {
        let check = guix_integration_check();

        if check.summary == "Guix substitute integration not confirmed" {
            assert_eq!(check.severity, DiagnosticSeverity::Error);
        }
    }

    #[test]
    fn diagnostics_format_groups_checks_with_summary() {
        let checks = vec![
            DiagnosticCheck {
                id: "identity",
                severity: DiagnosticSeverity::Ok,
                summary: "identity loaded".to_string(),
                detail: "peer id: example".to_string(),
            },
            DiagnosticCheck {
                id: "guix-integration",
                severity: DiagnosticSeverity::Error,
                summary: "Guix substitute integration not confirmed".to_string(),
                detail: "missing daemon environment".to_string(),
            },
            DiagnosticCheck {
                id: "shareable-address",
                severity: DiagnosticSeverity::Warning,
                summary: "no shareable peer address".to_string(),
                detail: "set external_addresses".to_string(),
            },
        ];

        let formatted = format_diagnostics(&checks);

        assert!(formatted.contains("guix-p2p doctor"));
        assert!(formatted.contains("status: not ready  1 ok, 1 warning(s), 1 error(s)"));
        assert!(formatted.contains("Guix substitute path"));
        assert!(formatted.contains("Connectivity"));
        assert!(formatted.contains("error guix-integration"));
    }

    #[test]
    fn diagnostics_format_can_emit_ansi_color() {
        let checks = vec![DiagnosticCheck {
            id: "identity",
            severity: DiagnosticSeverity::Ok,
            summary: "identity loaded".to_string(),
            detail: "peer id: example".to_string(),
        }];

        let formatted = format_diagnostics_with_color(&checks, true);

        assert!(formatted.contains("\x1b["));
        assert!(formatted.contains("identity"));
    }

    #[cfg(unix)]
    #[test]
    fn daemon_socket_check_accepts_live_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guix-p2p.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let check = daemon_socket_check(path.to_str().unwrap());

        assert_eq!(check.id, "daemon");
        assert_eq!(check.severity, DiagnosticSeverity::Ok);
    }

    #[test]
    fn daemon_socket_check_errors_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sock");

        let check = daemon_socket_check(path.to_str().unwrap());

        assert_eq!(check.id, "daemon");
        assert_eq!(check.severity, DiagnosticSeverity::Error);
        assert!(check.summary.contains("not running"));
    }

    #[cfg(unix)]
    #[test]
    fn daemon_socket_check_rejects_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guix-p2p.sock");
        std::fs::write(&path, "not a socket").unwrap();

        let check = daemon_socket_check(path.to_str().unwrap());

        assert_eq!(check.id, "daemon");
        assert_eq!(check.severity, DiagnosticSeverity::Error);
        assert!(check.summary.contains("not a Unix socket"));
    }

    #[test]
    fn diagnostic_report_marks_errors_and_warnings() {
        let peer = libp2p::PeerId::random().to_string();
        let mut config = config_with_addresses(vec![], vec![]);
        config.substitute_urls.clear();
        config.acl_path = "/definitely/missing/guix-p2p-acl".into();

        let report = diagnostic_report(&config, &peer);

        assert_eq!(report.peer_id, peer);
        assert!(report.has_errors);
        assert!(report.has_warnings);
        assert_eq!(report.connectivity.state, "local-only");
    }

    #[test]
    fn bootstrap_bundle_includes_paste_ready_config() {
        let peer = libp2p::PeerId::random().to_string();
        let config = config_with_addresses(
            vec!["/dns4/node.example.org/udp/6881/quic-v1".to_string()],
            vec![],
        );

        let bundle = bootstrap_bundle(&config, &peer);

        assert_eq!(bundle.bootstrap_peers, bundle.shareable_addresses);
        assert_eq!(
            bundle.config_snippet,
            format!("bootstrap_peers = \"/dns4/node.example.org/udp/6881/quic-v1/p2p/{peer}\"")
        );
    }

    #[test]
    fn bootstrap_bundle_includes_dashboard_url_when_enabled() {
        let peer = libp2p::PeerId::random().to_string();
        let mut config = config_with_addresses(vec![], vec![]);
        config.dashboard_enabled = true;
        config.dashboard_bind = "0.0.0.0".to_string();
        config.dashboard_port = 3030;

        let bundle = bootstrap_bundle(&config, &peer);

        assert_eq!(bundle.dashboard_url.as_deref(), Some("http://127.0.0.1:3030"));
    }
}
