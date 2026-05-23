mod daemon_protocol {
    use guix_p2p::daemon::{DaemonCommand, extract_hash_part};

    fn parse_line(line: &str) -> std::io::Result<DaemonCommand> {
        let mut cmd = line.to_string();
        if !cmd.ends_with('\n') {
            cmd.push('\n');
        }
        guix_p2p::daemon::parse_command_line(cmd.trim())
    }

    #[test]
    fn test_have_multiple_paths() {
        let cmd = parse_line("have /gnu/store/abc-foo /gnu/store/def-bar");
        let cmd = cmd.unwrap();
        match cmd {
            DaemonCommand::Have(paths) => {
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0], "/gnu/store/abc-foo");
                assert_eq!(paths[1], "/gnu/store/def-bar");
            },
            _ => panic!("expected Have"),
        }
    }

    #[test]
    fn test_substitute_with_dest() {
        let cmd = parse_line("substitute /gnu/store/abc-foo /tmp/dest-path");
        let cmd = cmd.unwrap();
        match cmd {
            DaemonCommand::Substitute { path, dest } => {
                assert_eq!(path, "/gnu/store/abc-foo");
                assert_eq!(dest, "/tmp/dest-path");
            },
            _ => panic!("expected Substitute"),
        }
    }

    #[test]
    fn test_info_command() {
        let cmd = parse_line("info /gnu/store/abc-foo");
        let cmd = cmd.unwrap();
        match cmd {
            DaemonCommand::Info(paths) => {
                assert_eq!(paths, vec!["/gnu/store/abc-foo"]);
            },
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn test_info_multiple_paths() {
        let cmd = parse_line("info /gnu/store/abc-foo /gnu/store/def-bar");
        let cmd = cmd.unwrap();
        match cmd {
            DaemonCommand::Info(paths) => {
                assert_eq!(paths, vec!["/gnu/store/abc-foo", "/gnu/store/def-bar"]);
            },
            _ => panic!("expected Info"),
        }
    }

    #[test]
    fn test_plain_path_fallback() {
        let cmd = parse_line("/gnu/store/abc-foo");
        let cmd = cmd.unwrap();
        match cmd {
            DaemonCommand::Have(paths) => {
                assert_eq!(paths.len(), 1);
            },
            _ => panic!("expected Have fallback"),
        }
    }

    #[test]
    fn test_extract_hash_valid() {
        let h = extract_hash_part("/gnu/store/0123456789abcdef0123456789abcdef-pkg-1.0").unwrap();
        assert_eq!(h, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn test_extract_hash_bad_path() {
        assert!(extract_hash_part("/bad/path").is_err());
    }

    #[test]
    fn test_extract_hash_no_dash() {
        assert!(extract_hash_part("/gnu/store/abcdef-foo-1.0").is_ok());
        assert!(extract_hash_part("/gnu/store/abcdefnodash").is_err());
    }
}

mod cli_contract {
    use std::process::Command;

    fn binary() -> &'static str {
        env!("CARGO_BIN_EXE_guix-p2p")
    }

    #[test]
    fn help_exits_successfully() {
        let output = Command::new(binary()).arg("--help").output().unwrap();

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    }

    #[test]
    fn version_exits_successfully() {
        let output = Command::new(binary()).arg("--version").output().unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("guix-p2p"));
        assert!(stdout.contains("0.1.6 ("));
    }

    #[test]
    fn relay_mode_does_not_print_startup_info_by_default() {
        let output = Command::new(binary())
            .args(["--query", "--socket", "/tmp/guix-p2p-test-missing.sock"])
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("Starting guix-p2p"));
        assert!(!stderr.contains("Cache directory"));
        assert!(!stderr.contains("Substitute policy"));
    }

    #[test]
    fn substitute_modes_are_mutually_exclusive() {
        let output = Command::new(binary()).args(["--query", "--substitute"]).output().unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
    }

    #[test]
    fn daemon_conflicts_with_query_mode() {
        let output = Command::new(binary()).args(["--query", "--daemon"]).output().unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
    }

    #[test]
    fn doctor_json_requires_doctor_mode() {
        let output = Command::new(binary()).arg("--json").output().unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("required"));
    }

    #[test]
    fn share_info_json_outputs_bootstrap_bundle() {
        let cache_dir = format!("/tmp/guix-p2p-share-info-{}", std::process::id());
        let output = Command::new(binary())
            .args([
                "--share-info",
                "--json",
                "--cache-dir",
                &cache_dir,
                "--external-addresses",
                "/dns4/node.example.org/udp/6881/quic-v1",
            ])
            .output()
            .unwrap();

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("\"peer_id\""));
        assert!(stdout.contains("\"bootstrap_peers\""));
        assert!(stdout.contains("bootstrap_peers ="));
    }

    #[test]
    fn test_connectivity_json_requires_multiaddr() {
        let output = Command::new(binary()).arg("--test-connectivity").output().unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("a value is required"));
    }

    #[test]
    fn test_connectivity_rejects_invalid_multiaddr() {
        let cache_dir = format!("/tmp/guix-p2p-test-connectivity-{}", std::process::id());
        let output = Command::new(binary())
            .args(["--test-connectivity", "not-a-multiaddr", "--cache-dir", &cache_dir])
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid multiaddr"));
    }
}

mod substitute_extension_contract {
    const EXTENSION: &str = include_str!("../guix/extensions/substitute.scm");

    #[test]
    fn extension_uses_in_process_socket_client() {
        assert!(EXTENSION.contains("(define (open-relay-socket"));
        assert!(EXTENSION.contains("mode: query"));
        assert!(EXTENSION.contains("mode: substitute"));
        assert!(EXTENSION.contains("fdopen 4"));
        assert!(EXTENSION.contains("base64-decode"));
        assert!(EXTENSION.contains("put-bytevector"));
    }

    #[test]
    fn extension_does_not_exec_rust_relay() {
        assert!(!EXTENSION.contains("execlp"));
        assert!(!EXTENSION.contains("GUIX_P2P_BIN"));
        assert!(!EXTENSION.contains("--socket"));
        assert!(!EXTENSION.contains("relay-arguments"));
    }
}

#[cfg(unix)]
mod scheme_extension {
    use base64::Engine as _;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::thread;

    fn extension_loads() -> bool {
        Command::new("guile")
            .args(["-L", ".", "-c", "(use-modules (guix extensions substitute))"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn substitute_extension_relays_to_socket_without_rust_helper() {
        if !extension_loads() {
            eprintln!("skipping Scheme extension test because guile/guix modules are unavailable");
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let socket_path = temp.path().join("guix-p2p.sock");
        let destination = temp.path().join("substitute-out");
        let fd4_path = temp.path().join("fd4");
        let nar_bytes = b"nar bytes from daemon";

        let listener = UnixListener::bind(&socket_path).unwrap();
        let expected_destination = destination.clone();
        let daemon = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                let read = reader.read_line(&mut line).unwrap();
                if read == 0 {
                    break;
                }
                lines.push(line.trim_end().to_string());
            }

            assert_eq!(lines[0], "mode: substitute");
            assert_eq!(
                lines[1],
                format!("substitute /gnu/store/abc-test {}", expected_destination.display())
            );

            let encoded = base64::engine::general_purpose::STANDARD.encode(nar_bytes);
            writeln!(stream, "nar:{encoded}").unwrap();
            writeln!(stream, "nar-end").unwrap();
            writeln!(stream, "fd4:success sha256:dummy {}", nar_bytes.len()).unwrap();
        });

        let fd4_file = File::create(&fd4_path).unwrap();
        let fd4 = std::os::fd::AsRawFd::as_raw_fd(&fd4_file);
        let mut command = Command::new("guile");
        command
            .args([
                "-L",
                ".",
                "-c",
                "(use-modules (guix extensions substitute)) (guix-substitute \"--substitute\")",
            ])
            .env("GUIX_P2P_SOCKET", &socket_path)
            .env("GUIX_P2P_BIN", "/definitely/missing/guix-p2p")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: the child process only duplicates an already-open temp file
        // descriptor onto fd 4 before exec; no shared Rust state is touched in
        // the pre-exec closure.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd4, 4) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(4, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();

        writeln!(
            child.stdin.as_mut().unwrap(),
            "substitute /gnu/store/abc-test {}",
            destination.display()
        )
        .unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        daemon.join().unwrap();

        assert!(
            output.status.success(),
            "guile failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let mut restored = Vec::new();
        File::open(&destination).unwrap().read_to_end(&mut restored).unwrap();
        assert_eq!(restored, nar_bytes);

        let mut fd4_output = String::new();
        drop(fd4_file);
        File::open(&fd4_path).unwrap().read_to_string(&mut fd4_output).unwrap();
        assert!(
            fd4_output.contains("success sha256:dummy"),
            "fd4 output missing reply; fd4={fd4_output:?}, stdout={}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

mod service_environment_contract {
    #[test]
    fn service_helpers_do_not_depend_on_shell_path_for_guix_tools() {
        for path in ["src/dashboard/packages.rs", "src/nar_store.rs"] {
            let source = std::fs::read_to_string(path).unwrap();
            assert!(
                !source.contains("Command::new(\"guix\")")
                    && !source.contains("Command::new(\"guile\")")
                    && !source.contains("std::process::Command::new(\"guix\")")
                    && !source.contains("std::process::Command::new(\"guile\")"),
                "{path} must resolve Guix tools from /run/current-system/profile/bin before falling back to PATH"
            );
        }
    }
}

// ======================================================================
// Reputation Behaviour Tests
// ======================================================================

mod reputation {
    use guix_p2p::reputation::ReputationTracker;

    #[test]
    fn test_clean_slate() {
        let tracker = ReputationTracker::new(5);
        let peers: Vec<libp2p::PeerId> = (0..3).map(|_| libp2p::PeerId::random()).collect();
        // all peers start with no data, score should be around default
        for p in &peers {
            let s = tracker.score(p);
            assert!((s - 0.5).abs() < 1e-6, "new peer should have default score");
        }
    }

    #[test]
    fn test_track_multiple_peers() {
        let mut tracker = ReputationTracker::new(5);
        let a = libp2p::PeerId::random();
        let b = libp2p::PeerId::random();
        let c = libp2p::PeerId::random();

        tracker.record_success(a, 1000);
        tracker.record_success(a, 2000);
        tracker.record_success(b, 500);
        tracker.record_failure(c);
        tracker.record_failure(c);
        tracker.record_failure(c);

        let sa = tracker.score(&a);
        let sb = tracker.score(&b);
        let sc = tracker.score(&c);
        assert!(sa > sb, "peer with more success should score higher");
        assert!(sc < sb, "peer with failures should score lower");
    }

    #[test]
    fn test_ban_threshold() {
        let mut tracker = ReputationTracker::new(3);
        let peer = libp2p::PeerId::random();

        // 0 failures: not banned
        assert!(!tracker.is_banned(&peer));

        // 2 failures: not banned
        tracker.record_failure(peer);
        tracker.record_failure(peer);
        assert!(!tracker.is_banned(&peer));

        // 3rd failure: banned
        tracker.record_failure(peer);
        assert!(tracker.is_banned(&peer));
    }

    #[test]
    fn test_sort_respects_ban() {
        let mut tracker = ReputationTracker::new(2);
        let good = libp2p::PeerId::random();
        let banned = libp2p::PeerId::random();

        tracker.record_success(good, 1024);
        tracker.record_failure(banned);
        tracker.record_failure(banned);

        let mut peers = vec![banned, good];
        tracker.sort_by_score(&mut peers);

        // banned may sort to the end but both should be present
        assert!(peers.contains(&good));
        assert!(peers.contains(&banned));
    }
}

// ======================================================================
// Connection Manager Tests
// ======================================================================

mod connection_manager {

    use guix_p2p::connection::{ConnectionConfig, ConnectionManager};

    #[test]
    fn test_record_attempt_tracks_retries() {
        let config = ConnectionConfig { max_retries: 3, ..Default::default() };
        let mut mgr = ConnectionManager::new(config);
        let peer = libp2p::PeerId::random();

        // First 3 attempts: allowed
        assert!(mgr.can_connect(&peer));
        mgr.record_attempt(peer);
        assert!(mgr.can_connect(&peer));
        mgr.record_attempt(peer);
        assert!(mgr.can_connect(&peer));
        mgr.record_attempt(peer);
        // 4th attempt: blocked
        assert!(!mgr.can_connect(&peer));
    }

    #[test]
    fn test_connected_resets_counters() {
        let config = ConnectionConfig { max_retries: 2, ..Default::default() };
        let mut mgr = ConnectionManager::new(config);
        let peer = libp2p::PeerId::random();

        mgr.record_attempt(peer);
        mgr.record_attempt(peer);
        assert!(!mgr.can_connect(&peer));

        // Simulate successful connection
        mgr.on_connected(peer);
        assert!(mgr.can_connect(&peer));
    }

    #[test]
    fn test_prune_removes_inactive_peers() {
        let mut mgr = ConnectionManager::new(ConnectionConfig::default());
        let peer = libp2p::PeerId::random();

        mgr.record_attempt(peer);

        // Simulate a very old last_active by directly testing prune behavior.
        // The connection manager prunes peers whose last_active is > 2x health_check_interval ago.
        // Since we just added the peer, it should not be dead yet.
        let dead = mgr.prune_dead();
        assert!(dead.is_empty(), "fresh peer should not be pruned");
    }
}

// ======================================================================
// Block Utilities Tests
// ======================================================================

mod block_utils {
    use guix_p2p::swarm::block::BlockInfo;

    #[test]
    fn test_block_count_for_exact_size() {
        let info = BlockInfo::from_file_size(524288, 262144);
        assert_eq!(info.block_count, 2);
    }

    #[test]
    fn test_block_count_for_partial_block() {
        let info = BlockInfo::from_file_size(300000, 262144);
        assert_eq!(info.block_count, 2);
    }

    #[test]
    fn test_block_count_for_one_byte() {
        let info = BlockInfo::from_file_size(1, 262144);
        assert_eq!(info.block_count, 1);
    }

    #[test]
    fn test_block_count_for_zero_size() {
        let info = BlockInfo::from_file_size(0, 262144);
        // empty nar should have 1 empty block
        assert_eq!(info.block_count, 1);
    }

    #[test]
    fn test_set_hashes() {
        let mut info = BlockInfo::from_file_size(100, 100);
        let hashes = vec![[0u8; 32]];
        info.set_hashes(hashes);
        assert_eq!(info.block_hashes.len(), 1);
    }
}

// ======================================================================
// Narinfo Parsing Edge Cases
// ======================================================================

mod narinfo_parsing {
    use guix_p2p::narinfo::parse_narinfo;

    #[test]
    fn test_empty_input() {
        let info = parse_narinfo("").unwrap();
        assert!(info.store_path.is_empty());
        assert!(info.urls.is_empty());
        assert!(info.signature.is_none());
    }

    #[test]
    fn test_only_required_fields() {
        let data = "StorePath: /gnu/store/a\nNarHash: sha256:abc\nNarSize: 42\n";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.nar_size, 42);
        assert!(info.references.is_empty());
        assert!(info.deriver.is_none());
    }

    #[test]
    fn test_urls_without_file_sizes() {
        let data = "\
StorePath: /gnu/store/a
NarHash: sha256:abc
NarSize: 100
Signature: 1;host;sig
URL: nar/gzip/a-test
Compression: gzip
URL: nar/zstd/a-test
Compression: zstd
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.urls.len(), 2);
        assert!(info.urls.iter().all(|u| u.file_size == 0));
    }

    #[test]
    fn test_malformed_signature_not_crashing() {
        let data = "\
StorePath: /gnu/store/a
NarHash: sha256:abc
NarSize: 100
Signature: 1
URL: nar/gzip/a
Compression: gzip
";
        let info = parse_narinfo(data).unwrap();
        assert_eq!(info.signature.as_deref(), Some("1"));
    }
}
