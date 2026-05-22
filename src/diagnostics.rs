use std::{
    net::{Ipv4Addr, Ipv6Addr},
    path::Path,
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

    checks.push(DiagnosticCheck {
        id: "identity",
        severity: DiagnosticSeverity::Ok,
        summary: "identity loaded".to_string(),
        detail: format!("peer id: {peer_id}"),
    });

    checks.push(DiagnosticCheck {
        id: "bootstrap-peers",
        severity: if summary.has_bootstrap_peers {
            DiagnosticSeverity::Ok
        } else {
            DiagnosticSeverity::Warning
        },
        summary: if summary.has_bootstrap_peers {
            "bootstrap peers configured".to_string()
        } else {
            "no bootstrap peers configured".to_string()
        },
        detail: if summary.has_bootstrap_peers {
            format!("{} bootstrap peer(s)", config.bootstrap_peers.len())
        } else {
            "configure bootstrap_peers or rely on LAN mDNS only".to_string()
        },
    });

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

    checks.push(path_check(
        "socket",
        Path::new(&config.socket_path),
        Path::new(&config.socket_path).exists(),
        "daemon socket exists",
        "daemon socket not present",
        "expected before relay/wrapper mode can route Guix substitute traffic through the daemon",
        DiagnosticSeverity::Warning,
    ));

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

fn ipv4_is_private_or_loopback(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local()
}

fn ipv6_is_private_or_loopback(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
}

/// Format diagnostics as a terminal-friendly report.
pub fn format_diagnostics(checks: &[DiagnosticCheck]) -> String {
    let mut out = String::new();
    for check in checks {
        let marker = match check.severity {
            DiagnosticSeverity::Ok => "ok",
            DiagnosticSeverity::Warning => "warn",
            DiagnosticSeverity::Error => "error",
        };
        out.push_str(&format!(
            "{marker:5} {:20} {}\n      {}\n",
            check.id, check.summary, check.detail
        ));
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
}
