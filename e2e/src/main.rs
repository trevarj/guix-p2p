use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

// ── CLI ──────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "guix-p2p-e2e", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run a real Guix p2p-only substitute smoke test
    ContainerSmoke {
        /// Guix package to build through the isolated daemon
        #[arg(long, default_value = "hello")]
        package: String,
        /// Pre-resolved /gnu/store path to seed instead of resolving the package first
        #[arg(long)]
        store_path: Option<String>,
        /// P2P transport for the two local nodes
        #[arg(long, value_enum, default_value_t = HarnessTransport::Tcp)]
        transport: HarnessTransport,
        /// Base directory for generated state and logs
        #[arg(long, default_value = "/tmp/guix-p2p-e2e")]
        base: PathBuf,
        /// Node A libp2p listen port
        #[arg(long, default_value_t = 6881)]
        node_a_port: u16,
        /// Node B libp2p listen port
        #[arg(long, default_value_t = 6882)]
        node_b_port: u16,
        /// Node A dashboard port
        #[arg(long, default_value_t = 3031)]
        node_a_dashboard_port: u16,
        /// Node B dashboard port
        #[arg(long, default_value_t = 3032)]
        node_b_dashboard_port: u16,
        /// Bind address for Node A and Node B dashboards
        #[arg(long, default_value = "127.0.0.1")]
        dashboard_bind: String,
        /// Existing guix-p2p binary to use instead of target/release/guix-p2p
        #[arg(long)]
        guix_p2p_bin: Option<PathBuf>,
        /// Run processes directly because a disposable VM is the isolation boundary
        #[arg(long)]
        vm_direct: bool,
        /// Keep daemons and dashboards running after validation until Ctrl-C
        #[arg(long)]
        hold: bool,
        /// Keep generated state under --base after completion
        #[arg(long)]
        keep_temp: bool,
    },
    /// Run controlled local substitute benchmarks
    Benchmark {
        /// Comma-separated Guix package names
        #[arg(long, value_delimiter = ',', default_value = "hello,git,emacs")]
        packages: Vec<String>,
        /// Benchmark modes
        #[arg(long, value_enum, value_delimiter = ',', default_value = "http,p2p-only,p2p-first")]
        modes: Vec<BenchmarkMode>,
        /// Iterations per package/mode
        #[arg(long, default_value_t = 3)]
        iterations: usize,
        /// P2P transport for P2P benchmark modes
        #[arg(long, value_enum, default_value_t = HarnessTransport::Tcp)]
        transport: HarnessTransport,
        /// Benchmark output and temporary state directory
        #[arg(long, default_value = "target/guix-p2p-bench")]
        base: PathBuf,
        /// Existing guix-p2p binary to use instead of target/release/guix-p2p
        #[arg(long)]
        guix_p2p_bin: Option<PathBuf>,
        /// Keep per-run temporary state directories
        #[arg(long)]
        keep_temp: bool,
    },
    /// Build, boot, and drive private-store VM E2E nodes
    Vm {
        /// State directory for images, disks, keys, logs, and node registry
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Guix system image size
        #[arg(long)]
        image_size: Option<String>,
        /// QEMU memory in MB
        #[arg(long)]
        memory: Option<u32>,
        /// QEMU CPU count
        #[arg(long)]
        cpus: Option<u32>,
        /// KVM mode: auto, true, or false
        #[arg(long)]
        enable_kvm: Option<String>,
        /// Forward dashboard host ports
        #[arg(long)]
        forward_dashboard: Option<bool>,
        /// Substitute URLs passed to guix system image and VM helpers
        #[arg(long)]
        substitute_urls: Option<String>,
        /// guix-p2p binary embedded in the image and pushed into VMs
        #[arg(long)]
        guix_p2p_binary: Option<PathBuf>,
        #[command(subcommand)]
        command: VmCommand,
    },
}

#[derive(Subcommand)]
enum VmCommand {
    /// Print the base qcow2 image derivation
    Derivation,
    /// Build the base qcow2 image
    Image,
    /// Reset one node disk or all known node disks from base.qcow2
    Reset {
        node: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Run a named node under QEMU in the background
    Run { node: String },
    /// Stop one named node or all known nodes
    Stop {
        node: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Show one named node or all known node statuses
    Status {
        node: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// SSH to a named node
    Ssh { node: String },
    /// Wait until SSH accepts connections on named nodes, or all known nodes
    WaitSsh { nodes: Vec<String> },
    /// Copy target/release/guix-p2p to named nodes, or all known nodes
    PushBinary {
        node: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Start a node as a seeder for a package and save seed metadata
    Seed {
        node: String,
        #[arg(default_value = "hello")]
        package: String,
    },
    /// Start a seedless node as the default DHT bootstrap peer
    Bootstrap { node: String },
    /// Print saved seed metadata as shell exports
    Env { node: Option<String> },
    /// Realize dependencies and remove the target output from a fetch node
    Remove {
        node: String,
        store_path: Option<String>,
        #[arg(long)]
        package: Option<String>,
    },
    /// Fetch the target output through the node's p2p-only Guix daemon
    Fetch {
        node: String,
        store_path: Option<String>,
        #[arg(long)]
        package: Option<String>,
    },
    /// Tail a node's guix-p2p log
    Logs { node: String },
    /// Tail a node's temporary guix-daemon log
    DaemonLog { node: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum HarnessTransport {
    Tcp,
    Quic,
}

impl HarnessTransport {
    fn listen_addr(self, port: u16) -> String {
        match self {
            HarnessTransport::Tcp => format!("/ip4/127.0.0.1/tcp/{port}"),
            HarnessTransport::Quic => format!("/ip4/127.0.0.1/udp/{port}/quic-v1"),
        }
    }
}

impl std::fmt::Display for HarnessTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessTransport::Tcp => write!(f, "tcp"),
            HarnessTransport::Quic => write!(f, "quic"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BenchmarkMode {
    Http,
    #[value(name = "p2p-only")]
    P2pOnly,
    #[value(name = "p2p-first")]
    P2pFirst,
}

impl BenchmarkMode {
    fn as_policy(self) -> Option<&'static str> {
        match self {
            BenchmarkMode::Http => None,
            BenchmarkMode::P2pOnly => Some("p2p-only"),
            BenchmarkMode::P2pFirst => Some("p2p-first"),
        }
    }
}

impl std::fmt::Display for BenchmarkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkMode::Http => write!(f, "http"),
            BenchmarkMode::P2pOnly => write!(f, "p2p-only"),
            BenchmarkMode::P2pFirst => write!(f, "p2p-first"),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "guix_p2p_e2e=info,guix_p2p=info,info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Commands::ContainerSmoke {
            package,
            store_path,
            transport,
            base,
            node_a_port,
            node_b_port,
            node_a_dashboard_port,
            node_b_dashboard_port,
            dashboard_bind,
            guix_p2p_bin,
            vm_direct,
            hold,
            keep_temp,
        } => {
            run_container_smoke(ContainerSmokeOptions {
                package,
                store_path,
                transport,
                base,
                node_a_port,
                node_b_port,
                node_a_dashboard_port,
                node_b_dashboard_port,
                dashboard_bind,
                guix_p2p_bin,
                vm_direct,
                hold,
                keep_temp,
            })
            .await
        },
        Commands::Benchmark {
            packages,
            modes,
            iterations,
            transport,
            base,
            guix_p2p_bin,
            keep_temp,
        } => {
            run_benchmark(BenchmarkOptions {
                packages,
                modes,
                iterations,
                transport,
                base,
                guix_p2p_bin,
                keep_temp,
            })
            .await
        },
        Commands::Vm {
            state_dir,
            image_size,
            memory,
            cpus,
            enable_kvm,
            forward_dashboard,
            substitute_urls,
            guix_p2p_binary,
            command,
        } => run_vm_command(VmOptions {
            state_dir,
            image_size,
            memory,
            cpus,
            enable_kvm,
            forward_dashboard,
            substitute_urls,
            guix_p2p_binary,
            command,
        }),
    }
}

// ── Real Guix smoke/benchmark harness ────────────────────────────────

struct ContainerSmokeOptions {
    package: String,
    store_path: Option<String>,
    transport: HarnessTransport,
    base: PathBuf,
    node_a_port: u16,
    node_b_port: u16,
    node_a_dashboard_port: u16,
    node_b_dashboard_port: u16,
    dashboard_bind: String,
    guix_p2p_bin: Option<PathBuf>,
    vm_direct: bool,
    hold: bool,
    keep_temp: bool,
}

struct BenchmarkOptions {
    packages: Vec<String>,
    modes: Vec<BenchmarkMode>,
    iterations: usize,
    transport: HarnessTransport,
    base: PathBuf,
    guix_p2p_bin: Option<PathBuf>,
    keep_temp: bool,
}

struct VmOptions {
    state_dir: Option<PathBuf>,
    image_size: Option<String>,
    memory: Option<u32>,
    cpus: Option<u32>,
    enable_kvm: Option<String>,
    forward_dashboard: Option<bool>,
    substitute_urls: Option<String>,
    guix_p2p_binary: Option<PathBuf>,
    command: VmCommand,
}

#[derive(Debug, Clone)]
struct VmConfig {
    state_dir: PathBuf,
    image_size: String,
    memory: u32,
    cpus: u32,
    enable_kvm: String,
    forward_dashboard: bool,
    substitute_urls: String,
    node_system: PathBuf,
    guix_p2p_binary: PathBuf,
    ssh_dir: PathBuf,
    ssh_host_key: PathBuf,
    ssh_host_key_pub: PathBuf,
    ssh_client_key: PathBuf,
    ssh_client_key_pub: PathBuf,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct VmRegistry {
    #[serde(default)]
    nodes: Vec<VmNode>,
    #[serde(default)]
    default_bootstrap: Option<VmBootstrap>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct VmNode {
    name: String,
    slug: String,
    ssh_port: u16,
    dashboard_port: u16,
    p2p_port: u16,
    disk: PathBuf,
    pid: Option<u32>,
    last_seed: Option<VmSeed>,
    last_fetch: Option<VmFetch>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct VmSeed {
    package: String,
    store_path: String,
    peer_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct VmFetch {
    from: String,
    package: String,
    store_path: String,
    peer_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct VmBootstrap {
    node: String,
    peer_id: String,
}

struct HarnessTools {
    guix: PathBuf,
    real_guix: PathBuf,
    raw_guix_daemon: PathBuf,
    guix_p2p: PathBuf,
    shell: PathBuf,
}

struct P2pBuildSpec<'a> {
    base: &'a std::path::Path,
    package: &'a str,
    store_path: &'a str,
    nar_hash: &'a str,
    transport: HarnessTransport,
    node_a_port: u16,
    node_b_port: u16,
    node_a_dashboard_port: u16,
    node_b_dashboard_port: u16,
    dashboard_bind: &'a str,
    node_b_policy: &'a str,
    strict_p2p_evidence: bool,
    hold_after_success: bool,
    vm_direct: bool,
    tools: &'a HarnessTools,
}

struct P2pBuildOutcome {
    elapsed_ms: u128,
    p2p_evidence: bool,
    nar_size: Option<u64>,
}

struct BenchmarkPackage {
    name: String,
    store_path: String,
    nar_hash: String,
}

struct BenchmarkRecord {
    package: String,
    store_path: String,
    nar_hash: String,
    nar_size: Option<u64>,
    mode: BenchmarkMode,
    iteration: usize,
    elapsed_ms: Option<u128>,
    success: bool,
    p2p_evidence: bool,
    run_dir: PathBuf,
    error: Option<String>,
}

struct ManagedChild {
    label: String,
    child: std::process::Child,
}

#[derive(Default)]
struct ProcessSet {
    children: Vec<ManagedChild>,
}

impl ProcessSet {
    fn spawn_logged(
        &mut self,
        label: &str,
        command: &mut std::process::Command,
        log_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .with_context(|| format!("failed to open log {}", log_path.display()))?;
        let stderr = log.try_clone().context("failed to clone log file")?;
        tracing::debug!("spawning {label}: {:?}", command);
        let child = command
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("failed to spawn {label}"))?;
        self.children.push(ManagedChild { label: label.to_string(), child });
        Ok(())
    }
}

impl Drop for ProcessSet {
    fn drop(&mut self) {
        for managed in &mut self.children {
            match managed.child.try_wait() {
                Ok(Some(_)) => {},
                Ok(None) => {
                    let _ = managed.child.kill();
                    let _ = managed.child.wait();
                },
                Err(e) => {
                    tracing::debug!("failed to inspect child {}: {}", managed.label, e);
                },
            }
        }
    }
}

fn run_vm_command(opts: VmOptions) -> anyhow::Result<()> {
    let config = VmConfig::from_options(&opts)?;
    match opts.command {
        VmCommand::Derivation => vm_image_derivation(&config),
        VmCommand::Image => vm_build_image(&config),
        VmCommand::Reset { node, all } => vm_reset(&config, node.as_deref(), all),
        VmCommand::Run { node } => {
            let mut registry = VmRegistry::load(&config)?;
            let node = registry.ensure_node(&config, &node)?.clone();
            registry.save(&config)?;
            vm_run_node(&config, &node)
        },
        VmCommand::Stop { node, all } => vm_stop(&config, node.as_deref(), all),
        VmCommand::Status { node, all } => vm_status(&config, node.as_deref(), all),
        VmCommand::Ssh { node } => {
            let registry = VmRegistry::load(&config)?;
            let node = registry.node(&node)?;
            vm_ssh(&config, node)
        },
        VmCommand::WaitSsh { nodes } => vm_wait_ssh(&config, &nodes),
        VmCommand::PushBinary { node, all } => vm_push_binary(&config, node.as_deref(), all),
        VmCommand::Seed { node, package } => vm_seed(&config, &node, &package),
        VmCommand::Bootstrap { node } => vm_bootstrap(&config, &node),
        VmCommand::Env { node } => vm_print_env(&config, node.as_deref()),
        VmCommand::Remove { node, store_path, package } => {
            vm_remove(&config, &node, store_path.as_deref(), package.as_deref())
        },
        VmCommand::Fetch { node, store_path, package } => {
            vm_fetch(&config, &node, store_path.as_deref(), package.as_deref())
        },
        VmCommand::Logs { node } => vm_tail_log(&config, &node, "guix-p2p"),
        VmCommand::DaemonLog { node } => vm_tail_log(&config, &node, "daemon"),
    }
}

impl VmConfig {
    fn from_options(opts: &VmOptions) -> anyhow::Result<Self> {
        let root = project_root();
        let state_dir = opts
            .state_dir
            .clone()
            .or_else(|| std::env::var_os("GUIX_P2P_E2E_DIR").map(PathBuf::from))
            .or_else(|| std::env::var_os("GUIX_P2P_E2E_PRIVATE_DIR").map(PathBuf::from))
            .unwrap_or_else(|| root.join("target/guix-p2p-e2e"));
        let state_dir = absolutize_path(&root, &state_dir);
        let ssh_dir = state_dir.join("ssh");
        let ssh_host_key = std::env::var_os("GUIX_P2P_E2E_SSH_HOST_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|| ssh_dir.join("ssh_host_ed25519_key"));
        let ssh_host_key_pub = std::env::var_os("GUIX_P2P_E2E_SSH_HOST_KEY_PUB")
            .map(PathBuf::from)
            .unwrap_or_else(|| ssh_host_key.with_extension("pub"));
        let ssh_client_key = std::env::var_os("GUIX_P2P_E2E_SSH_CLIENT_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|| ssh_dir.join("e2e_ed25519"));
        let ssh_client_key_pub = std::env::var_os("GUIX_P2P_E2E_SSH_AUTHORIZED_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|| ssh_client_key.with_extension("pub"));
        Ok(Self {
            state_dir,
            image_size: opts
                .image_size
                .clone()
                .or_else(|| std::env::var("GUIX_P2P_E2E_IMAGE_SIZE").ok())
                .unwrap_or_else(|| "8G".to_string()),
            memory: opts.memory.or_else(|| env_u32("GUIX_P2P_E2E_VM_MEMORY")).unwrap_or(2048),
            cpus: opts.cpus.or_else(|| env_u32("GUIX_P2P_E2E_VM_CPUS")).unwrap_or(2),
            enable_kvm: opts
                .enable_kvm
                .clone()
                .or_else(|| std::env::var("GUIX_P2P_E2E_ENABLE_KVM").ok())
                .unwrap_or_else(|| "auto".to_string()),
            forward_dashboard: opts
                .forward_dashboard
                .or_else(|| env_bool("GUIX_P2P_E2E_FORWARD_DASHBOARD"))
                .unwrap_or(true),
            substitute_urls: opts
                .substitute_urls
                .clone()
                .or_else(|| std::env::var("GUIX_P2P_E2E_SUBSTITUTE_URLS").ok())
                .unwrap_or_else(|| {
                    "https://ci.guix.gnu.org https://bordeaux.guix.gnu.org".to_string()
                }),
            node_system: root.join("guix/e2e-node.scm"),
            guix_p2p_binary: opts
                .guix_p2p_binary
                .clone()
                .or_else(|| std::env::var_os("GUIX_P2P_E2E_BINARY").map(PathBuf::from))
                .unwrap_or_else(|| root.join("target/release/guix-p2p")),
            ssh_dir,
            ssh_host_key,
            ssh_host_key_pub,
            ssh_client_key,
            ssh_client_key_pub,
        })
    }

    fn registry_path(&self) -> PathBuf {
        self.state_dir.join("nodes.json")
    }

    fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    fn base_image_root(&self) -> PathBuf {
        self.state_dir.join("base-image")
    }

    fn base_disk(&self) -> PathBuf {
        self.state_dir.join("base.qcow2")
    }

    fn proof_env_path(&self, node: &VmNode) -> PathBuf {
        self.state_dir.join(format!("{}.env", node.slug))
    }
}

impl VmRegistry {
    fn load(config: &VmConfig) -> anyhow::Result<Self> {
        let path = config.registry_path();
        if !path.exists() {
            return Ok(Self { nodes: Vec::new(), default_bootstrap: None });
        }
        let bytes =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    fn save(&self, config: &VmConfig) -> anyhow::Result<()> {
        std::fs::create_dir_all(&config.state_dir)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(config.registry_path(), bytes)?;
        Ok(())
    }

    fn node(&self, name: &str) -> anyhow::Result<&VmNode> {
        self.nodes.iter().find(|node| node.name == name || node.slug == name).ok_or_else(|| {
            anyhow::anyhow!("unknown VM node {name}; run it first with: vm run {name}")
        })
    }

    fn node_mut(&mut self, name: &str) -> anyhow::Result<&mut VmNode> {
        self.nodes.iter_mut().find(|node| node.name == name || node.slug == name).ok_or_else(|| {
            anyhow::anyhow!("unknown VM node {name}; run it first with: vm run {name}")
        })
    }

    fn ensure_node(&mut self, config: &VmConfig, name: &str) -> anyhow::Result<&mut VmNode> {
        if let Some(idx) = self.nodes.iter().position(|node| node.name == name || node.slug == name)
        {
            return Ok(&mut self.nodes[idx]);
        }
        if !config.base_disk().is_file() {
            anyhow::bail!("base disk is missing; build it first with: vm image");
        }
        let slug = unique_slug(name, self);
        let (ssh_port, dashboard_port, p2p_port) = self.allocate_ports();
        let disk = config.state_dir.join(format!("{slug}.qcow2"));
        tracing::info!("creating VM node {name}; disk={}", disk.display());
        std::fs::copy(config.base_disk(), &disk)?;
        make_user_writable(&disk)?;
        self.nodes.push(VmNode {
            name: name.to_string(),
            slug,
            ssh_port,
            dashboard_port,
            p2p_port,
            disk,
            pid: None,
            last_seed: None,
            last_fetch: None,
        });
        Ok(self.nodes.last_mut().expect("node was just pushed"))
    }

    fn allocate_ports(&self) -> (u16, u16, u16) {
        let mut ssh = 2221;
        let mut dashboard = 3031;
        let mut p2p = 6881;
        loop {
            let used = self.nodes.iter().any(|node| {
                node.ssh_port == ssh || node.dashboard_port == dashboard || node.p2p_port == p2p
            });
            if !used
                && tcp_port_available(ssh)
                && tcp_port_available(dashboard)
                && tcp_port_available(p2p)
            {
                return (ssh, dashboard, p2p);
            }
            ssh += 1;
            dashboard += 1;
            p2p += 1;
        }
    }

    fn latest_seed(&self) -> anyhow::Result<(&VmNode, &VmSeed)> {
        self.nodes
            .iter()
            .rev()
            .find_map(|node| node.last_seed.as_ref().map(|seed| (node, seed)))
            .ok_or_else(|| anyhow::anyhow!("no saved seed found; run: vm seed <node> hello"))
    }

    fn bootstrap_multiaddr(&self) -> anyhow::Result<Option<String>> {
        let Some(bootstrap) = &self.default_bootstrap else {
            return Ok(None);
        };
        let node = self.node(&bootstrap.node)?;
        Ok(Some(vm_peer_multiaddr(node, &bootstrap.peer_id)))
    }
}

fn vm_image_derivation(config: &VmConfig) -> anyhow::Result<()> {
    ensure_vm_prereqs(config)?;
    let mut command = std::process::Command::new("guix");
    command
        .arg("system")
        .arg("image")
        .arg("--derivation")
        .arg("--image-type=qcow2")
        .arg(format!("--image-size={}", config.image_size))
        .arg(format!("--substitute-urls={}", config.substitute_urls))
        .arg(&config.node_system)
        .env("GUIX_P2P_E2E_BINARY", &config.guix_p2p_binary)
        .env("GUIX_P2P_E2E_SSH_HOST_KEY", &config.ssh_host_key)
        .env("GUIX_P2P_E2E_SSH_HOST_KEY_PUB", &config.ssh_host_key_pub)
        .env("GUIX_P2P_E2E_SSH_AUTHORIZED_KEY", &config.ssh_client_key_pub);
    let output = checked_output(&mut command, "guix system image --derivation")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

fn vm_build_image(config: &VmConfig) -> anyhow::Result<()> {
    ensure_vm_prereqs(config)?;
    std::fs::create_dir_all(config.logs_dir())?;
    let root = config.base_image_root();
    if root.exists() {
        std::fs::remove_file(&root).or_else(|_| std::fs::remove_dir_all(&root)).ok();
    }
    let log_path = config.logs_dir().join("base-image-build.log");
    let log =
        std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(&log_path)?;
    let stderr = log.try_clone()?;
    let mut command = std::process::Command::new("guix");
    command
        .arg("system")
        .arg("image")
        .arg("--image-type=qcow2")
        .arg(format!("--image-size={}", config.image_size))
        .arg(format!("--substitute-urls={}", config.substitute_urls))
        .arg(format!("--root={}", root.display()))
        .arg(&config.node_system)
        .env("GUIX_P2P_E2E_BINARY", &config.guix_p2p_binary)
        .env("GUIX_P2P_E2E_SSH_HOST_KEY", &config.ssh_host_key)
        .env("GUIX_P2P_E2E_SSH_HOST_KEY_PUB", &config.ssh_host_key_pub)
        .env("GUIX_P2P_E2E_SSH_AUTHORIZED_KEY", &config.ssh_client_key_pub)
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(stderr));
    tracing::info!("building base image; log={}", log_path.display());
    let status = command.status().context("failed to run guix system image")?;
    if !status.success() {
        anyhow::bail!(
            "base image build failed with {status}; log tail:\n{}",
            read_tail(&log_path, 120)
        );
    }
    let source = root.canonicalize()?;
    std::fs::copy(&source, config.base_disk())?;
    make_user_writable(&config.base_disk())?;
    println!("{}", config.base_disk().display());
    Ok(())
}

fn vm_reset(config: &VmConfig, node: Option<&str>, all: bool) -> anyhow::Result<()> {
    if !config.base_disk().is_file() {
        anyhow::bail!("base disk is missing; build it first with: vm image");
    }
    let mut registry = VmRegistry::load(config)?;
    let targets: Vec<String> = if all || node.is_none() {
        registry.nodes.iter().map(|node| node.name.clone()).collect()
    } else {
        vec![registry.node(node.expect("checked above"))?.name.clone()]
    };
    if targets.is_empty() {
        anyhow::bail!("no VM nodes are registered yet");
    }
    let clear_bootstrap = registry
        .default_bootstrap
        .as_ref()
        .is_some_and(|bootstrap| targets.iter().any(|name| name == &bootstrap.node));
    for name in targets {
        let target = registry.node_mut(&name)?;
        if let Some(pid) = target.pid {
            stop_pid(pid).with_context(|| format!("failed to stop {}", target.name))?;
        }
        std::fs::copy(config.base_disk(), &target.disk)?;
        make_user_writable(&target.disk)?;
        target.pid = None;
        target.last_seed = None;
        target.last_fetch = None;
        println!("{}", target.disk.display());
    }
    if clear_bootstrap {
        registry.default_bootstrap = None;
    }
    registry.save(config)?;
    Ok(())
}

fn vm_run_node(config: &VmConfig, node: &VmNode) -> anyhow::Result<()> {
    if is_pid_running(node.pid) {
        tracing::info!("{} already running with pid {}", node.name, node.pid.unwrap());
        return Ok(());
    }
    std::fs::create_dir_all(config.logs_dir())?;
    let serial = config.logs_dir().join(format!("{}-serial.log", node.slug));
    let log_path = config.logs_dir().join(format!("{}-qemu.log", node.slug));
    let log =
        std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(&log_path)?;
    let stderr = log.try_clone()?;
    let mut command = qemu_command(config, node, &serial)?;
    command.stdout(std::process::Stdio::from(log)).stderr(std::process::Stdio::from(stderr));
    tracing::info!(
        "starting {}; qemu log={} serial={}",
        node.name,
        log_path.display(),
        serial.display()
    );
    let child = command.spawn().with_context(|| format!("failed to start {}", node.name))?;
    let pid = child.id();
    std::mem::forget(child);
    let mut registry = VmRegistry::load(config)?;
    registry.node_mut(&node.name)?.pid = Some(pid);
    registry.save(config)?;
    println!("{} pid={pid}", node.name);
    Ok(())
}

fn qemu_command(
    config: &VmConfig,
    node: &VmNode,
    serial: &std::path::Path,
) -> anyhow::Result<std::process::Command> {
    let mut args = Vec::<String>::new();
    match config.enable_kvm.as_str() {
        "true" | "yes" | "1" => args.push("-enable-kvm".to_string()),
        "false" | "no" | "0" => {},
        "auto" => {
            if std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm").is_ok() {
                args.push("-enable-kvm".to_string());
            } else {
                tracing::info!("KVM unavailable; using QEMU software emulation");
            }
        },
        other => anyhow::bail!("invalid KVM mode {other}; expected auto, true, or false"),
    }
    let mut netdev =
        format!("user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:{}-:22", node.ssh_port);
    if config.forward_dashboard {
        netdev.push_str(&format!(",hostfwd=tcp:127.0.0.1:{}-:3031", node.dashboard_port));
    }
    netdev.push_str(&format!(",hostfwd=tcp:127.0.0.1:{}-:6881", node.p2p_port));
    args.extend([
        "-m".to_string(),
        config.memory.to_string(),
        "-smp".to_string(),
        config.cpus.to_string(),
        "-nographic".to_string(),
        "-serial".to_string(),
        format!("file:{}", serial.display()),
        "-drive".to_string(),
        format!("file={},if=virtio,format=qcow2", node.disk.display()),
        "-nic".to_string(),
        netdev,
    ]);
    if let Some(qemu) = find_on_path("qemu-system-x86_64") {
        let mut command = std::process::Command::new(qemu);
        command.args(args);
        return Ok(command);
    }
    let mut command = std::process::Command::new("guix");
    command.arg("shell").arg("qemu").arg("--").arg("qemu-system-x86_64").args(args);
    Ok(command)
}

fn vm_stop(config: &VmConfig, node: Option<&str>, all: bool) -> anyhow::Result<()> {
    let mut registry = VmRegistry::load(config)?;
    let names: Vec<String> = if all || node.is_none() {
        registry.nodes.iter().map(|node| node.name.clone()).collect()
    } else {
        vec![registry.node(node.expect("checked above"))?.name.clone()]
    };
    for name in names {
        let target = registry.node_mut(&name)?;
        if let Some(pid) = target.pid {
            if is_pid_running(Some(pid)) {
                stop_pid(pid)?;
                println!("{} stopped", target.name);
            }
            target.pid = None;
        } else {
            println!("{} not running", target.name);
        }
    }
    registry.save(config)
}

fn vm_status(config: &VmConfig, node: Option<&str>, all: bool) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let targets: Vec<&VmNode> = if all || node.is_none() {
        registry.nodes.iter().collect()
    } else {
        vec![registry.node(node.expect("checked above"))?]
    };
    if targets.is_empty() {
        println!("no VM nodes registered");
        return Ok(());
    }
    for target in targets {
        let running = is_pid_running(target.pid);
        let role = if registry
            .default_bootstrap
            .as_ref()
            .is_some_and(|bootstrap| bootstrap.node == target.name)
        {
            "bootstrap"
        } else if target.last_seed.is_some() {
            "seed"
        } else if target.last_fetch.is_some() {
            "fetch"
        } else {
            "-"
        };
        println!(
            "{} slug={} role={} running={} pid={} ssh={} dashboard={} p2p={} disk={}",
            target.name,
            target.slug,
            role,
            running,
            target.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
            target.ssh_port,
            target.dashboard_port,
            target.p2p_port,
            target.disk.display()
        );
    }
    Ok(())
}

fn vm_ssh(config: &VmConfig, node: &VmNode) -> anyhow::Result<()> {
    ensure_ssh_client_key(config)?;
    let status = ssh_command(config, node)
        .status()
        .with_context(|| format!("failed to run ssh for {}", node.name))?;
    if !status.success() {
        anyhow::bail!("ssh {} failed with {status}", node.name);
    }
    Ok(())
}

fn vm_wait_ssh(config: &VmConfig, nodes: &[String]) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let targets: Vec<&VmNode> = if nodes.is_empty() {
        registry.nodes.iter().collect()
    } else {
        nodes.iter().map(|name| registry.node(name)).collect::<anyhow::Result<Vec<_>>>()?
    };
    for node in targets {
        wait_ssh(config, node)?;
        println!("{} SSH ready on port {}", node.name, node.ssh_port);
    }
    Ok(())
}

fn vm_push_binary(config: &VmConfig, node: Option<&str>, all: bool) -> anyhow::Result<()> {
    ensure_guix_p2p_binary(config)?;
    let registry = VmRegistry::load(config)?;
    let targets: Vec<&VmNode> = if all || node.is_none() {
        registry.nodes.iter().collect()
    } else {
        vec![registry.node(node.expect("checked above"))?]
    };
    let libgcrypt_runtime = guix_build_last_path("libgcrypt")?;
    for target in targets {
        push_binary_to_node(config, target, &libgcrypt_runtime)?;
        println!("pushed binary to {}:/tmp/guix-p2p", target.name);
    }
    Ok(())
}

fn vm_seed(config: &VmConfig, name: &str, package: &str) -> anyhow::Result<()> {
    let mut registry = VmRegistry::load(config)?;
    let node = registry.node(name)?.clone();
    let bootstrap = registry.bootstrap_multiaddr()?;
    let bootstrap_env = bootstrap
        .as_ref()
        .map(|addr| format!("GUIX_P2P_E2E_A_BOOTSTRAP={} ", shell_quote(addr)))
        .unwrap_or_default();
    let output = ssh_run(
        config,
        &node,
        &format!(
            "GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p {bootstrap_env}guix-p2p-e2e-node-a {}",
            shell_quote(package)
        ),
    )?;
    print!("{output}");
    let store_path = parse_key_line(&output, "store_path")
        .ok_or_else(|| anyhow::anyhow!("seed output did not include store_path"))?;
    let peer_id = parse_key_line(&output, "peer_id")
        .ok_or_else(|| anyhow::anyhow!("seed output did not include peer_id"))?;
    let seed = VmSeed { package: package.to_string(), store_path, peer_id };
    registry.node_mut(name)?.last_seed = Some(seed.clone());
    registry.save(config)?;
    write_vm_env(config, &node, &seed)?;
    println!("\n# Optional shell exports:");
    print_vm_env(&seed);
    println!("# Or load them with: cargo run -p guix-p2p-e2e -- vm env {}", node.name);
    Ok(())
}

fn vm_bootstrap(config: &VmConfig, name: &str) -> anyhow::Result<()> {
    let mut registry = VmRegistry::load(config)?;
    let node = registry.node(name)?.clone();
    let command = format!(
        "GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p GUIX_P2P_E2E_BOOTSTRAP_LISTEN=/ip4/0.0.0.0/tcp/6881 \
         GUIX_P2P_E2E_BOOTSTRAP_DASHBOARD_PORT=3031 {}",
        bootstrap_node_command()
    );
    let output = ssh_run(config, &node, &command)?;
    print!("{output}");
    let peer_id = parse_key_line(&output, "peer_id")
        .ok_or_else(|| anyhow::anyhow!("bootstrap output did not include peer_id"))?;
    registry.default_bootstrap = Some(VmBootstrap { node: node.name.clone(), peer_id });
    registry.save(config)?;
    println!("bootstrap={}", registry.bootstrap_multiaddr()?.unwrap_or_default());
    Ok(())
}

fn vm_print_env(config: &VmConfig, node: Option<&str>) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let seed = match node {
        Some(name) => registry.node(name)?.last_seed.as_ref(),
        None => registry.nodes.iter().rev().find_map(|node| node.last_seed.as_ref()),
    }
    .ok_or_else(|| anyhow::anyhow!("no saved seed found; run: vm seed <node> hello"))?;
    print_vm_env(seed);
    Ok(())
}

fn vm_remove(
    config: &VmConfig,
    name: &str,
    store_path: Option<&str>,
    package: Option<&str>,
) -> anyhow::Result<()> {
    let mut registry = VmRegistry::load(config)?;
    let node = registry.node(name)?.clone();
    let target = resolve_fetch_target(&registry, &node, store_path, package)?;
    registry.node_mut(name)?.last_fetch = Some(target.clone());
    registry.save(config)?;
    let output = ssh_run(
        config,
        &node,
        &format!(
            "set -eu; guix build --no-grafts {}; guix gc -D {}; test ! -e {} && echo \
             TARGET_ABSENT_AFTER_DELETE",
            shell_quote(&target.package),
            shell_quote(&target.store_path),
            shell_quote(&target.store_path)
        ),
    )?;
    print!("{output}");
    Ok(())
}

fn vm_start_daemon(config: &VmConfig, name: &str) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let node = registry.node(name)?;
    let remote = r#"
set -eu
cat > /tmp/e2e-guix-wrapper <<'EOF'
#!/bin/sh
set -eu
SOCKET=/tmp/guix-p2p-b/guix-p2p.sock
GUIX_P2P=/tmp/guix-p2p
REAL_GUIX=/run/current-system/profile/bin/guix

case "${1-}" in
  substitute)
    shift
    case "${1-}" in
      --query|--substitute)
        exec "$GUIX_P2P" "$@" --socket "$SOCKET"
        ;;
      *)
        exec "$REAL_GUIX" substitute "$@"
        ;;
    esac
    ;;
  *)
    exec "$REAL_GUIX" "$@"
    ;;
esac
EOF
chmod +x /tmp/e2e-guix-wrapper
printf "e2e\n" | sudo -S sh -c '
mount -o remount,rw /gnu/store
kill $(cat /tmp/e2e-guix-daemon.pid 2>/dev/null) 2>/dev/null || true
rm -f /tmp/e2e-guix-daemon.sock /tmp/e2e-guix-daemon.log /tmp/e2e-guix-daemon.pid
GUIX=/tmp/e2e-guix-wrapper /run/current-system/profile/bin/guix-daemon \
  --disable-chroot \
  --build-users-group=guixbuild \
  --max-jobs=0 \
  --listen=/tmp/e2e-guix-daemon.sock \
  > /tmp/e2e-guix-daemon.log 2>&1 &
echo $! > /tmp/e2e-guix-daemon.pid
'
i=0
while [ ! -S /tmp/e2e-guix-daemon.sock ]; do
    i=$((i + 1))
    if [ "$i" -gt 30 ]; then
        echo DAEMON_SOCKET_TIMEOUT
        printf "e2e\n" | sudo -S cat /tmp/e2e-guix-daemon.log
        exit 1
    fi
    sleep 1
done
echo GUIX_DAEMON_SOCKET=/tmp/e2e-guix-daemon.sock
"#;
    let output = ssh_run(config, node, remote)?;
    print!("{output}");
    Ok(())
}

fn vm_start_fetch_p2p(config: &VmConfig, name: &str, target: &VmFetch) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let node = registry.node(name)?;
    let bootstrap = registry
        .bootstrap_multiaddr()?
        .ok_or_else(|| anyhow::anyhow!("no bootstrap node configured; run: vm bootstrap <node>"))?;
    let command = format!(
        "GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p GUIX_P2P_E2E_B_LISTEN=/ip4/0.0.0.0/tcp/6881 \
         GUIX_P2P_E2E_B_DASHBOARD_PORT=3031 GUIX_P2P_E2E_B_BOOTSTRAP={} guix-p2p-e2e-node-b {} {}",
        shell_quote(&bootstrap),
        shell_quote(&target.store_path),
        shell_quote(&target.peer_id)
    );
    let output = ssh_run(config, node, &command)?;
    print!("{output}");
    Ok(())
}

fn vm_fetch(
    config: &VmConfig,
    name: &str,
    store_path: Option<&str>,
    package: Option<&str>,
) -> anyhow::Result<()> {
    let mut registry = VmRegistry::load(config)?;
    let node = registry.node(name)?.clone();
    let target = resolve_fetch_target(&registry, &node, store_path, package)?;
    registry.node_mut(name)?.last_fetch = Some(target.clone());
    registry.save(config)?;
    vm_start_fetch_p2p(config, name, &target)?;
    vm_start_daemon(config, name)?;
    let output = ssh_run(
        config,
        &node,
        &format!(
            "set -eu; test ! -e {}; GUIX_DAEMON_SOCKET=/tmp/e2e-guix-daemon.sock guix build \
             --no-grafts {}; test -d {} && echo IMPORTED_OUTPUT_IN_NODE_STORE",
            shell_quote(&target.store_path),
            shell_quote(&target.package),
            shell_quote(&target.store_path)
        ),
    )?;
    print!("{output}");
    Ok(())
}

fn vm_tail_log(config: &VmConfig, name: &str, kind: &str) -> anyhow::Result<()> {
    let registry = VmRegistry::load(config)?;
    let node = registry.node(name)?;
    let path = match kind {
        "daemon" => "/tmp/e2e-guix-daemon.log",
        _ if registry
            .default_bootstrap
            .as_ref()
            .is_some_and(|bootstrap| bootstrap.node == node.name) =>
        {
            "/tmp/guix-p2p-bootstrap.log"
        },
        _ if node.last_fetch.is_some() => "/tmp/guix-p2p-b.log",
        _ => "/tmp/guix-p2p-a.log",
    };
    let status = ssh_command(config, node)
        .arg(format!("tail -f {}", shell_quote(path)))
        .status()
        .with_context(|| format!("failed to tail log on {}", node.name))?;
    if !status.success() {
        anyhow::bail!("tail log failed with {status}");
    }
    Ok(())
}

async fn run_container_smoke(opts: ContainerSmokeOptions) -> anyhow::Result<()> {
    let tools = prepare_harness_tools(opts.guix_p2p_bin.as_deref())?;
    let base = absolutize_path(&project_root(), &opts.base);
    reset_dir(&base)?;
    std::fs::create_dir_all(base.join("logs"))?;
    ensure_container_guix_store_writable(&tools, &base, opts.vm_direct)?;

    let store_path = match opts.store_path {
        Some(path) => {
            if !std::path::Path::new(&path).exists() {
                anyhow::bail!("--store-path does not exist: {path}");
            }
            path
        },
        None => {
            tracing::info!("resolving Guix package {}", opts.package);
            resolve_package(&tools.guix, &opts.package)?
        },
    };
    let nar_hash = compute_nar_hash(&tools.guix, &store_path)?;

    tracing::info!("seed store path: {}", store_path);
    tracing::info!("nar hash: {}", nar_hash);

    let outcome = run_p2p_build(P2pBuildSpec {
        base: &base,
        package: &opts.package,
        store_path: &store_path,
        nar_hash: &nar_hash,
        transport: opts.transport,
        node_a_port: opts.node_a_port,
        node_b_port: opts.node_b_port,
        node_a_dashboard_port: opts.node_a_dashboard_port,
        node_b_dashboard_port: opts.node_b_dashboard_port,
        dashboard_bind: &opts.dashboard_bind,
        node_b_policy: "p2p-only",
        strict_p2p_evidence: true,
        hold_after_success: opts.hold,
        vm_direct: opts.vm_direct,
        tools: &tools,
    })
    .await?;

    tracing::info!(
        "container smoke passed: package={} elapsed={}ms p2p_evidence={} logs={}",
        opts.package,
        outcome.elapsed_ms,
        outcome.p2p_evidence,
        base.join("logs").display()
    );
    tracing::info!("seed store path: {}", store_path);
    tracing::info!("nar hash: {}", nar_hash);
    tracing::info!(
        "node A dashboard: http://{}:{}",
        opts.dashboard_bind,
        opts.node_a_dashboard_port
    );
    tracing::info!(
        "node B dashboard: http://{}:{}",
        opts.dashboard_bind,
        opts.node_b_dashboard_port
    );
    if opts.keep_temp {
        tracing::info!("kept generated state under {}", base.display());
    }
    Ok(())
}

async fn run_benchmark(opts: BenchmarkOptions) -> anyhow::Result<()> {
    if opts.iterations == 0 {
        anyhow::bail!("--iterations must be greater than zero");
    }
    if opts.packages.is_empty() {
        anyhow::bail!("--packages must contain at least one package");
    }
    if opts.modes.is_empty() {
        anyhow::bail!("--modes must contain at least one mode");
    }

    let tools = prepare_harness_tools(opts.guix_p2p_bin.as_deref())?;
    let base = absolutize_path(&project_root(), &opts.base);
    std::fs::create_dir_all(&base)?;
    let tmp_root = base.join("tmp");
    reset_dir(&tmp_root)?;
    ensure_container_guix_store_writable(&tools, &tmp_root, false)?;

    let mut packages = Vec::new();
    for package in &opts.packages {
        tracing::info!("resolving benchmark package {}", package);
        let store_path = resolve_package(&tools.guix, package)?;
        let nar_hash = compute_nar_hash(&tools.guix, &store_path)?;
        packages.push(BenchmarkPackage { name: package.clone(), store_path, nar_hash });
    }

    let mut records = Vec::new();
    let mut failures = Vec::new();
    let mut port_seed = 41_000_u16;
    let mut dash_seed = 31_000_u16;

    for package in &packages {
        for mode in &opts.modes {
            for iteration in 1..=opts.iterations {
                let run_dir = tmp_root.join(format!(
                    "{}-{}-{}",
                    sanitize_name(&package.name),
                    mode,
                    iteration
                ));
                reset_dir(&run_dir)?;
                std::fs::create_dir_all(run_dir.join("logs"))?;

                tracing::info!(
                    "benchmark package={} mode={} iteration={}/{}",
                    package.name,
                    mode,
                    iteration,
                    opts.iterations
                );

                let result = match mode {
                    BenchmarkMode::Http => run_http_benchmark(&run_dir, &package.name, &tools)
                        .map(|elapsed_ms| (elapsed_ms, false, None)),
                    BenchmarkMode::P2pOnly | BenchmarkMode::P2pFirst => {
                        let node_a_port = reserve_transport_port(opts.transport, &mut port_seed)?;
                        let node_b_port = reserve_transport_port(opts.transport, &mut port_seed)?;
                        let node_a_dashboard_port = reserve_tcp_port(&mut dash_seed)?;
                        let node_b_dashboard_port = reserve_tcp_port(&mut dash_seed)?;
                        let policy = mode.as_policy().expect("p2p mode has a policy");
                        run_p2p_build(P2pBuildSpec {
                            base: &run_dir,
                            package: &package.name,
                            store_path: &package.store_path,
                            nar_hash: &package.nar_hash,
                            transport: opts.transport,
                            node_a_port,
                            node_b_port,
                            node_a_dashboard_port,
                            node_b_dashboard_port,
                            dashboard_bind: "127.0.0.1",
                            node_b_policy: policy,
                            strict_p2p_evidence: *mode == BenchmarkMode::P2pOnly,
                            hold_after_success: false,
                            vm_direct: false,
                            tools: &tools,
                        })
                        .await
                        .map(|outcome| (outcome.elapsed_ms, outcome.p2p_evidence, outcome.nar_size))
                    },
                };

                match result {
                    Ok((elapsed_ms, p2p_evidence, nar_size)) => records.push(BenchmarkRecord {
                        package: package.name.clone(),
                        store_path: package.store_path.clone(),
                        nar_hash: package.nar_hash.clone(),
                        nar_size,
                        mode: *mode,
                        iteration,
                        elapsed_ms: Some(elapsed_ms),
                        success: true,
                        p2p_evidence,
                        run_dir: run_dir.clone(),
                        error: None,
                    }),
                    Err(e) => {
                        let message = e.to_string();
                        failures.push(format!(
                            "{} {} iteration {}: {}",
                            package.name, mode, iteration, message
                        ));
                        records.push(BenchmarkRecord {
                            package: package.name.clone(),
                            store_path: package.store_path.clone(),
                            nar_hash: package.nar_hash.clone(),
                            nar_size: None,
                            mode: *mode,
                            iteration,
                            elapsed_ms: None,
                            success: false,
                            p2p_evidence: false,
                            run_dir: run_dir.clone(),
                            error: Some(message),
                        });
                    },
                }

                if !opts.keep_temp {
                    let _ = std::fs::remove_dir_all(&run_dir);
                }
            }
        }
    }

    let csv_path = base.join("results.csv");
    write_benchmark_csv(&csv_path, &records)?;
    write_benchmark_report(
        &project_root().join("docs/benchmark-results.md"),
        &records,
        &packages,
        opts.transport,
        opts.iterations,
    )?;

    tracing::info!("benchmark CSV: {}", csv_path.display());
    tracing::info!("benchmark report: docs/benchmark-results.md");

    if !opts.keep_temp {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "benchmark completed with {} failed run(s); see docs/benchmark-results.md",
            failures.len()
        );
    }
}

fn prepare_harness_tools(guix_p2p_bin: Option<&std::path::Path>) -> anyhow::Result<HarnessTools> {
    let guix = find_on_path("guix").context("missing required command: guix")?;
    let shell = find_on_path("sh")
        .or_else(|| {
            canonicalize_existing(std::path::Path::new("/run/current-system/profile/bin/sh")).ok()
        })
        .context("missing required command: sh")?;
    let real_guix = canonicalize_existing(&guix)?;
    let raw_guix_daemon = resolve_raw_guix_daemon()?;
    let guix_p2p = match guix_p2p_bin {
        Some(path) => canonicalize_existing(path)?,
        None => {
            let cargo = find_on_path("cargo").context("missing required command: cargo")?;
            build_release_binary(&cargo)?;
            project_root().join("target/release/guix-p2p")
        },
    };
    if !guix_p2p.is_file() {
        anyhow::bail!("guix-p2p binary not found at {}", guix_p2p.display());
    }
    Ok(HarnessTools { guix, real_guix, raw_guix_daemon, guix_p2p, shell })
}

fn build_release_binary(cargo: &std::path::Path) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(cargo);
    command.current_dir(project_root()).args(["build", "--release"]);
    checked_status(command, "cargo build --release")
}

fn guix_container_command(
    tools: &HarnessTools,
    base: &std::path::Path,
    vm_direct: bool,
) -> std::process::Command {
    if vm_direct || std::env::var_os("GUIX_P2P_E2E_NO_GUIX_SHELL").is_some() {
        if let Some(env) = find_on_path("env") {
            return std::process::Command::new(env);
        }
        let profile_env = std::path::Path::new("/run/current-system/profile/bin/env");
        if profile_env.exists() {
            return std::process::Command::new(profile_env);
        }
        return std::process::Command::new("env");
    }

    let mut command = std::process::Command::new(&tools.guix);
    command
        .arg("shell")
        .arg("-C")
        .arg("-N")
        .arg("--writable-root")
        .arg(format!("--share={}", base.display()));

    let root = project_root();
    if root.exists() {
        command.arg(format!("--share={}", root.display()));
    }

    if std::path::Path::new("/etc/guix").exists() {
        command.arg("--expose=/etc/guix");
    }
    if std::path::Path::new("/var/guix").exists() {
        command.arg("--expose=/var/guix");
    }

    command.arg("--");
    command
}

fn ensure_container_guix_store_writable(
    tools: &HarnessTools,
    base: &std::path::Path,
    vm_direct: bool,
) -> anyhow::Result<()> {
    if vm_direct || std::env::var_os("GUIX_P2P_E2E_NO_GUIX_SHELL").is_some() {
        let probe = std::path::Path::new("/gnu/store/.guix-p2p-e2e-write-test");
        std::fs::write(probe, b"probe").map_err(|e| {
            anyhow::anyhow!(
                "Guix VM E2E requires /gnu/store to be writable. The direct preflight failed: {e}"
            )
        })?;
        std::fs::remove_file(probe).ok();
        return Ok(());
    }

    let mut command = guix_container_command(tools, base, vm_direct);
    command.args([
        "/bin/sh",
        "-c",
        "test -w /gnu/store || { echo '/gnu/store is not writable inside guix shell -CN' >&2; \
         exit 1; }",
    ]);
    checked_status(command, "guix shell -CN writable /gnu/store preflight").map_err(|e| {
        anyhow::anyhow!(
            "Guix container E2E requires /gnu/store to be writable inside `guix shell -CN`. The \
             container preflight failed: {e}"
        )
    })
}

async fn run_p2p_build(spec: P2pBuildSpec<'_>) -> anyhow::Result<P2pBuildOutcome> {
    let logs_dir = spec.base.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let node_a_dir = spec.base.join("node-a");
    let node_b_dir = spec.base.join("node-b");
    let node_a_cache = node_a_dir.join("cache");
    let node_b_cache = node_b_dir.join("cache");
    let node_a_config_home = node_a_dir.join("config-home");
    let node_b_config_home = node_b_dir.join("config-home");
    let node_a_socket = node_a_dir.join("guix-p2p.sock");
    let node_b_socket = node_b_dir.join("guix-p2p.sock");
    let daemon_socket = node_b_dir.join("guix-daemon.sock");
    let wrapper_path = node_b_dir.join("guix-wrapper.sh");

    std::fs::create_dir_all(&node_a_cache)?;
    std::fs::create_dir_all(&node_b_cache)?;
    std::fs::create_dir_all(&node_a_config_home)?;
    std::fs::create_dir_all(&node_b_config_home)?;

    let node_a_addr = spec.transport.listen_addr(spec.node_a_port);
    let node_b_addr = spec.transport.listen_addr(spec.node_b_port);

    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_a_config_home,
        listen_addr: &node_a_addr,
        cache_dir: &node_a_cache,
        socket_path: &node_a_socket,
        substitute_policy: "p2p-only",
        min_providers: 1,
        dashboard_port: spec.node_a_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: None,
        seed_paths: &[spec.store_path],
    })?;
    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_b_config_home,
        listen_addr: &node_b_addr,
        cache_dir: &node_b_cache,
        socket_path: &node_b_socket,
        substitute_policy: spec.node_b_policy,
        min_providers: 1,
        dashboard_port: spec.node_b_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: None,
        seed_paths: &[],
    })?;

    write_wrapper(
        &wrapper_path,
        &node_b_socket,
        &spec.tools.guix_p2p,
        &spec.tools.real_guix,
        &spec.tools.shell,
    )?;

    let mut processes = ProcessSet::default();

    let mut node_a_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    node_a_cmd
        .arg(&spec.tools.guix_p2p)
        .arg("--daemon")
        .arg("--listen-addr")
        .arg(&node_a_addr)
        .arg("--cache-dir")
        .arg(&node_a_cache)
        .arg("--socket")
        .arg(&node_a_socket)
        .arg("--dashboard")
        .arg("--dashboard-port")
        .arg(spec.node_a_dashboard_port.to_string())
        .arg("--dashboard-bind")
        .arg(spec.dashboard_bind)
        .arg("--policy")
        .arg("p2p-only")
        .arg("--seed")
        .arg(spec.store_path)
        .env("HOME", &node_a_dir)
        .env("XDG_CONFIG_HOME", &node_a_config_home)
        .env("RUST_LOG", "guix_p2p=trace,info");
    let node_a_log = logs_dir.join("node-a.log");
    processes.spawn_logged("node-a", &mut node_a_cmd, &node_a_log)?;
    wait_dashboard(spec.node_a_dashboard_port, "node A", Some(&node_a_log))?;

    let node_a_status = dashboard_json(spec.node_a_dashboard_port, "/api/status")?;
    let node_a_peer = json_string(&node_a_status, "peer_id")
        .context("node A dashboard did not expose peer_id")?;
    let bootstrap = format!("{node_a_addr}/p2p/{node_a_peer}");

    maybe_remove_seed_store_path(spec.store_path)?;

    write_node_config(NodeConfigSpec {
        xdg_config_home: &node_b_config_home,
        listen_addr: &node_b_addr,
        cache_dir: &node_b_cache,
        socket_path: &node_b_socket,
        substitute_policy: spec.node_b_policy,
        min_providers: 1,
        dashboard_port: spec.node_b_dashboard_port,
        dashboard_bind: spec.dashboard_bind,
        bootstrap_peers: Some(&bootstrap),
        seed_paths: &[],
    })?;

    let mut node_b_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    node_b_cmd
        .arg(&spec.tools.guix_p2p)
        .arg("--daemon")
        .arg("--listen-addr")
        .arg(&node_b_addr)
        .arg("--cache-dir")
        .arg(&node_b_cache)
        .arg("--socket")
        .arg(&node_b_socket)
        .arg("--dashboard")
        .arg("--dashboard-port")
        .arg(spec.node_b_dashboard_port.to_string())
        .arg("--dashboard-bind")
        .arg(spec.dashboard_bind)
        .arg("--policy")
        .arg(spec.node_b_policy)
        .arg("--bootstrap-peers")
        .arg(&bootstrap)
        .env("HOME", &node_b_dir)
        .env("XDG_CONFIG_HOME", &node_b_config_home)
        .env("RUST_LOG", "guix_p2p=trace,info");
    let node_b_log = logs_dir.join("node-b.log");
    processes.spawn_logged("node-b", &mut node_b_cmd, &node_b_log)?;
    wait_dashboard(spec.node_b_dashboard_port, "node B", Some(&node_b_log))?;
    wait_unix_socket(&node_b_socket, "node B relay socket")?;

    let guix_state = prepare_guix_daemon_state(&node_b_dir)?;
    let mut daemon_cmd = guix_container_command(spec.tools, spec.base, spec.vm_direct);
    daemon_cmd
        .arg(&spec.tools.raw_guix_daemon)
        .arg("--disable-chroot")
        .arg("--max-jobs=0")
        .arg(format!("--listen={}", daemon_socket.display()))
        .env("HOME", &node_b_dir)
        .env("GUIX", &wrapper_path)
        .env("GUIX_STATE_DIRECTORY", &guix_state.state_dir)
        .env("GUIX_CONFIGURATION_DIRECTORY", &guix_state.config_dir);
    processes.spawn_logged("guix-daemon", &mut daemon_cmd, &logs_dir.join("guix-daemon.log"))?;
    wait_unix_socket(&daemon_socket, "isolated guix-daemon socket")?;

    let elapsed_ms = run_guix_build_logged(
        spec.tools,
        spec.base,
        spec.package,
        &daemon_socket,
        &logs_dir.join("build.log"),
        spec.vm_direct,
    )?;

    let seeds = dashboard_json(spec.node_a_dashboard_port, "/api/seeds")?;
    let nar_size = validate_seeds(&seeds, spec.nar_hash)?;
    let _catalog = wait_for_catalog_entry(
        spec.node_b_dashboard_port,
        spec.store_path,
        spec.nar_hash,
        std::time::Duration::from_secs(30),
    )?;

    let node_a_log = std::fs::read_to_string(logs_dir.join("node-a.log")).unwrap_or_default();
    let node_b_log = std::fs::read_to_string(logs_dir.join("node-b.log")).unwrap_or_default();
    let p2p_evidence =
        contains_any(&node_a_log, &["Served block request", "serving ", "BlockServed"]);
    let node_b_success = contains_any(
        &node_b_log,
        &["Substitute download succeeded", "DownloadSucceeded", "download-succeeded"],
    );

    if spec.strict_p2p_evidence && !p2p_evidence {
        anyhow::bail!(
            "node A log did not show block serving evidence; see {}",
            logs_dir.join("node-a.log").display()
        );
    }
    if !node_b_success {
        anyhow::bail!(
            "node B log did not show a successful substitute download; see {}",
            logs_dir.join("node-b.log").display()
        );
    }
    if spec.node_b_policy == "p2p-only" {
        if !contains_any(&node_b_log, &["p2p-only", "P2pOnly"]) {
            anyhow::bail!(
                "node B log did not show p2p-only substitute handling; see {}",
                logs_dir.join("node-b.log").display()
            );
        }
        if contains_any(&node_b_log, &["falling back to HTTP", "Attempting HTTP nar download"]) {
            anyhow::bail!(
                "node B used an HTTP nar fallback in p2p-only mode; see {}",
                logs_dir.join("node-b.log").display()
            );
        }
    }

    if spec.hold_after_success {
        tracing::info!("strict P2P smoke proof passed; holding daemons until Ctrl-C");
        tracing::info!(
            "node A dashboard: http://{}:{}",
            spec.dashboard_bind,
            spec.node_a_dashboard_port
        );
        tracing::info!(
            "node B dashboard: http://{}:{}",
            spec.dashboard_bind,
            spec.node_b_dashboard_port
        );
        tokio::signal::ctrl_c().await.context("failed to wait for Ctrl-C")?;
        tracing::info!("Ctrl-C received; stopping smoke daemons");
    }

    drop(processes);

    Ok(P2pBuildOutcome { elapsed_ms, p2p_evidence, nar_size })
}

fn maybe_remove_seed_store_path(store_path: &str) -> anyhow::Result<()> {
    if std::env::var_os("GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A").is_none() {
        return Ok(());
    }
    let path = std::path::Path::new(store_path);
    if !path.starts_with("/gnu/store") || !path.exists() {
        return Ok(());
    }
    tracing::info!("removing seeded store path before builder run: {}", store_path);
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
        .with_context(|| format!("failed to remove seeded store path {store_path}"))
}

fn run_http_benchmark(
    run_dir: &std::path::Path,
    package: &str,
    tools: &HarnessTools,
) -> anyhow::Result<u128> {
    let logs_dir = run_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    let daemon_socket = run_dir.join("guix-daemon.sock");
    let guix_state = prepare_guix_daemon_state(run_dir)?;
    let mut processes = ProcessSet::default();

    let mut daemon_cmd = guix_container_command(tools, run_dir, false);
    daemon_cmd
        .arg(&tools.raw_guix_daemon)
        .arg("--disable-chroot")
        .arg("--max-jobs=0")
        .arg(format!("--listen={}", daemon_socket.display()))
        .env("HOME", run_dir)
        .env("GUIX", &tools.real_guix)
        .env("GUIX_STATE_DIRECTORY", &guix_state.state_dir)
        .env("GUIX_CONFIGURATION_DIRECTORY", &guix_state.config_dir);
    processes.spawn_logged("guix-daemon", &mut daemon_cmd, &logs_dir.join("guix-daemon.log"))?;
    wait_unix_socket(&daemon_socket, "isolated guix-daemon socket")?;
    let elapsed_ms = run_guix_build_logged(
        tools,
        run_dir,
        package,
        &daemon_socket,
        &logs_dir.join("build.log"),
        false,
    )?;
    drop(processes);
    Ok(elapsed_ms)
}

struct NodeConfigSpec<'a> {
    xdg_config_home: &'a std::path::Path,
    listen_addr: &'a str,
    cache_dir: &'a std::path::Path,
    socket_path: &'a std::path::Path,
    substitute_policy: &'a str,
    min_providers: usize,
    dashboard_port: u16,
    dashboard_bind: &'a str,
    bootstrap_peers: Option<&'a str>,
    seed_paths: &'a [&'a str],
}

fn write_node_config(spec: NodeConfigSpec<'_>) -> anyhow::Result<()> {
    let config_dir = spec.xdg_config_home.join("guix-p2p");
    std::fs::create_dir_all(&config_dir)?;
    let mut toml = String::new();
    toml.push_str(&format!("listen_addr = {}\n", toml_string(spec.listen_addr)));
    toml.push_str(&format!("cache_dir = {}\n", toml_string(&spec.cache_dir.display().to_string())));
    toml.push_str(&format!(
        "socket_path = {}\n",
        toml_string(&spec.socket_path.display().to_string())
    ));
    toml.push_str(&format!("substitute_policy = {}\n", toml_string(spec.substitute_policy)));
    toml.push_str(&format!("min_providers = {}\n", spec.min_providers));
    toml.push_str("request_timeout_secs = 60\n");
    toml.push_str("stall_timeout_secs = 30\n");
    toml.push_str("dashboard_enabled = true\n");
    toml.push_str(&format!("dashboard_port = {}\n", spec.dashboard_port));
    toml.push_str(&format!("dashboard_bind = {}\n", toml_string(spec.dashboard_bind)));
    toml.push_str("substitute_urls = \"https://bordeaux.guix.gnu.org,https://ci.guix.gnu.org\"\n");
    if let Some(peers) = spec.bootstrap_peers {
        toml.push_str(&format!("bootstrap_peers = {}\n", toml_string(peers)));
    }
    toml.push_str("seed_paths = [");
    for (idx, path) in spec.seed_paths.iter().enumerate() {
        if idx > 0 {
            toml.push_str(", ");
        }
        toml.push_str(&toml_string(path));
    }
    toml.push_str("]\n");
    std::fs::write(config_dir.join("config.toml"), toml)?;
    Ok(())
}

struct GuixDaemonState {
    state_dir: PathBuf,
    config_dir: PathBuf,
}

fn prepare_guix_daemon_state(base: &std::path::Path) -> anyhow::Result<GuixDaemonState> {
    let state_dir = base.join("state");
    let config_dir = base.join("etc");
    for rel in ["db", "daemon-socket", "gcroots", "profiles", "substitute", "temproots", "userpool"]
    {
        std::fs::create_dir_all(state_dir.join(rel))?;
    }
    std::fs::create_dir_all(&config_dir)?;
    let host_acl = std::path::Path::new("/etc/guix/acl");
    if host_acl.exists() {
        let _ = std::fs::copy(host_acl, config_dir.join("acl"));
    }
    Ok(GuixDaemonState { state_dir, config_dir })
}

fn write_wrapper(
    wrapper: &std::path::Path,
    socket: &std::path::Path,
    guix_p2p: &std::path::Path,
    real_guix: &std::path::Path,
    shell: &std::path::Path,
) -> anyhow::Result<()> {
    let content = format!(
        r#"#!{}
SOCKET={}
GUIX_P2P={}
REAL_GUIX={}

case "${{1-}}" in
    substitute)
        shift
        case "${{1-}}" in
            --query|--substitute)
                if [ -S "$SOCKET" ]; then
                    exec "$GUIX_P2P" "$@" --socket "$SOCKET"
                fi
                exec "$REAL_GUIX" substitute "$@"
                ;;
            *)
                exec "$REAL_GUIX" substitute "$@"
                ;;
        esac
        ;;
    *)
        exec "$REAL_GUIX" "$@"
        ;;
esac
"#,
        shell.display(),
        shell_quote(&socket.display().to_string()),
        shell_quote(&guix_p2p.display().to_string()),
        shell_quote(&real_guix.display().to_string())
    );
    std::fs::write(wrapper, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(wrapper)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(wrapper, permissions)?;
    }
    Ok(())
}

fn run_guix_build_logged(
    tools: &HarnessTools,
    base: &std::path::Path,
    package: &str,
    daemon_socket: &std::path::Path,
    log_path: &std::path::Path,
    vm_direct: bool,
) -> anyhow::Result<u128> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(log_path)?;
    let stderr = log.try_clone()?;
    let started = std::time::Instant::now();
    let mut command = guix_container_command(tools, base, vm_direct);
    command.arg(&tools.guix).arg("build").arg(package).env("GUIX_DAEMON_SOCKET", daemon_socket);
    tracing::debug!("running build command: {:?}", command);
    let status = command
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(stderr))
        .status()
        .with_context(|| format!("failed to run guix build {package}"))?;
    let elapsed_ms = started.elapsed().as_millis();
    if !status.success() {
        anyhow::bail!(
            "guix build {} failed with {}; log tail:\n{}",
            package,
            status,
            read_tail(log_path, 80)
        );
    }
    Ok(elapsed_ms)
}

fn wait_dashboard(
    port: u16,
    label: &str,
    log_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        match dashboard_json(port, "/api/status") {
            Ok(_) => return Ok(()),
            Err(e) if std::time::Instant::now() < deadline => {
                tracing::debug!("waiting for {label} dashboard: {e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
            },
            Err(e) => {
                let tail = log_path.map(|path| read_tail(path, 80)).unwrap_or_default();
                anyhow::bail!(
                    "timed out waiting for {label} dashboard on {port}: {e}\n{} log tail:\n{}",
                    label,
                    tail
                );
            },
        }
    }
}

fn wait_unix_socket(path: &std::path::Path, label: &str) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if is_unix_socket(path) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for {label} at {}", path.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn dashboard_json(port: u16, path: &str) -> anyhow::Result<serde_json::Value> {
    let body = http_get_body(port, path)?;
    serde_json::from_str(&body).with_context(|| format!("dashboard returned invalid JSON: {body}"))
}

fn http_get_body(port: u16, path: &str) -> anyhow::Result<String> {
    use std::io::{Read, Write};

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
        anyhow::bail!("dashboard HTTP response was not 200: {}", first_line(&response));
    }
    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("dashboard response did not contain a body"))?;
    Ok(body.to_string())
}

fn wait_for_catalog_entry(
    dashboard_port: u16,
    store_path: &str,
    nar_hash: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<serde_json::Value> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let catalog = dashboard_json(dashboard_port, "/api/catalog")?;
        if catalog_entry_matches(&catalog, store_path, nar_hash) {
            return Ok(catalog);
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "node B catalog did not include {} or nar hash {}; last catalog: {}",
                store_path,
                nar_hash,
                catalog
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn validate_seeds(seeds: &serde_json::Value, nar_hash: &str) -> anyhow::Result<Option<u64>> {
    let entries =
        seeds.as_array().ok_or_else(|| anyhow::anyhow!("/api/seeds did not return an array"))?;
    if entries.is_empty() {
        anyhow::bail!("node A /api/seeds was empty");
    }
    for entry in entries {
        if entry.get("nar_hash").and_then(serde_json::Value::as_str) == Some(nar_hash) {
            return Ok(entry.get("nar_size").and_then(serde_json::Value::as_u64));
        }
    }
    anyhow::bail!("node A /api/seeds did not include seeded nar {}", nar_hash);
}

fn catalog_entry_matches(catalog: &serde_json::Value, store_path: &str, nar_hash: &str) -> bool {
    let Some(entries) = catalog.as_array() else {
        return false;
    };
    let prefixed_nar_hash = format!("sha256:{nar_hash}");
    let hash_part = store_hash_part(store_path);
    entries.iter().any(|entry| {
        entry.get("store_path").and_then(serde_json::Value::as_str) == Some(store_path)
            || entry.get("nar_hash").and_then(serde_json::Value::as_str) == Some(&prefixed_nar_hash)
            || hash_part.as_deref().is_some_and(|h| {
                entry.get("hash_part").and_then(serde_json::Value::as_str) == Some(h)
            })
    })
}

fn resolve_package(guix: &std::path::Path, package: &str) -> anyhow::Result<String> {
    let output = checked_output(
        std::process::Command::new(guix).arg("build").arg(package),
        &format!("guix build {package}"),
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("/gnu/store/"))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("guix build {} did not print a store path", package))
}

fn compute_nar_hash(guix: &std::path::Path, store_path: &str) -> anyhow::Result<String> {
    let output = checked_output(
        std::process::Command::new(guix).args(["hash", "-S", "nar", "-f", "hex", store_path]),
        &format!("guix hash -S nar -f hex {store_path}"),
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn checked_status(mut command: std::process::Command, description: &str) -> anyhow::Result<()> {
    let output = command.output().with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} failed with {}\nstdout:\n{}\nstderr:\n{}",
            description,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn checked_output(
    command: &mut std::process::Command,
    description: &str,
) -> anyhow::Result<std::process::Output> {
    let output = command.output().with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        anyhow::bail!(
            "{} failed with {}\nstdout:\n{}\nstderr:\n{}",
            description,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn resolve_raw_guix_daemon() -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir("/gnu/store").context("failed to read /gnu/store")? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.contains("-guix-") {
            continue;
        }
        let candidate = entry.path().join("bin/guix-daemon");
        if candidate.is_file() && is_elf(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not find raw ELF guix-daemon under /gnu/store"))
}

fn is_elf(path: &std::path::Path) -> bool {
    use std::io::Read;

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF"
}

fn is_unix_socket(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(path).map(|m| m.file_type().is_socket()).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn reserve_transport_port(transport: HarnessTransport, next: &mut u16) -> anyhow::Result<u16> {
    match transport {
        HarnessTransport::Tcp => reserve_tcp_port(next),
        HarnessTransport::Quic => reserve_udp_port(next),
    }
}

fn reserve_tcp_port(next: &mut u16) -> anyhow::Result<u16> {
    while *next < 60_000 {
        let port = *next;
        *next += 1;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("could not reserve a TCP port")
}

fn reserve_udp_port(next: &mut u16) -> anyhow::Result<u16> {
    while *next < 60_000 {
        let port = *next;
        *next += 1;
        if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("could not reserve a UDP port")
}

fn write_benchmark_csv(path: &std::path::Path, records: &[BenchmarkRecord]) -> anyhow::Result<()> {
    let mut csv = String::from(
        "package,store_path,nar_hash,nar_size,mode,iteration,elapsed_ms,success,p2p_evidence,\
         run_dir,error\n",
    );
    for record in records {
        csv.push_str(&csv_row(&[
            record.package.clone(),
            record.store_path.clone(),
            record.nar_hash.clone(),
            record.nar_size.map(|n| n.to_string()).unwrap_or_default(),
            record.mode.to_string(),
            record.iteration.to_string(),
            record.elapsed_ms.map(|n| n.to_string()).unwrap_or_default(),
            record.success.to_string(),
            record.p2p_evidence.to_string(),
            record.run_dir.display().to_string(),
            record.error.clone().unwrap_or_default(),
        ]));
        csv.push('\n');
    }
    std::fs::write(path, csv)?;
    Ok(())
}

fn write_benchmark_report(
    path: &std::path::Path,
    records: &[BenchmarkRecord],
    packages: &[BenchmarkPackage],
    transport: HarnessTransport,
    iterations: usize,
) -> anyhow::Result<()> {
    let mut report = String::new();
    report.push_str("# Benchmark Results\n\n");
    report.push_str("Generated by `guix-p2p-e2e benchmark`.\n\n");
    report.push_str("## Host\n\n");
    report.push_str(&format!("- Date: {}\n", unix_timestamp()));
    report.push_str(&format!("- Platform: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
    report.push_str(&format!("- Transport: {transport}\n"));
    report.push_str(&format!("- Iterations: {iterations}\n"));
    report.push_str(&format!("- Rust: {}\n\n", rust_version()));

    report.push_str("## Packages\n\n");
    report.push_str("| Package | Store path | Nar hash | Nar size |\n");
    report.push_str("|---------|------------|----------|----------|\n");
    for package in packages {
        let nar_size = records
            .iter()
            .find(|r| r.package == package.name && r.nar_size.is_some())
            .and_then(|r| r.nar_size)
            .map(format_bytes)
            .unwrap_or_else(|| "unknown".to_string());
        report.push_str(&format!(
            "| {} | `{}` | `{}` | {} |\n",
            package.name, package.store_path, package.nar_hash, nar_size
        ));
    }

    report.push_str("\n## Runs\n\n");
    report.push_str("| Package | Mode | Iteration | Elapsed | Success | P2P evidence |\n");
    report.push_str("|---------|------|-----------|---------|---------|--------------|\n");
    for record in records {
        let elapsed = record
            .elapsed_ms
            .map(|ms| format!("{:.3}s", ms as f64 / 1000.0))
            .unwrap_or_else(|| "failed".to_string());
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            record.package,
            record.mode,
            record.iteration,
            elapsed,
            record.success,
            record.p2p_evidence
        ));
    }

    report.push_str("\n## Medians\n\n");
    report.push_str(
        "| Package | Mode | Median elapsed | Successful runs | P2P evidence observed |\n",
    );
    report.push_str(
        "|---------|------|----------------|-----------------|-----------------------|\n",
    );
    for package in packages {
        let mut modes: Vec<BenchmarkMode> =
            records.iter().filter(|r| r.package == package.name).map(|r| r.mode).collect();
        modes.sort_by_key(|m| m.to_string());
        modes.dedup();
        for mode in modes {
            let subset: Vec<&BenchmarkRecord> =
                records.iter().filter(|r| r.package == package.name && r.mode == mode).collect();
            let median = median_ms(subset.iter().filter_map(|r| r.elapsed_ms).collect());
            let successes = subset.iter().filter(|r| r.success).count();
            let p2p = subset.iter().any(|r| r.p2p_evidence);
            report.push_str(&format!(
                "| {} | {} | {} | {}/{} | {} |\n",
                package.name,
                mode,
                median
                    .map(|ms| format!("{:.3}s", ms as f64 / 1000.0))
                    .unwrap_or_else(|| "n/a".to_string()),
                successes,
                subset.len(),
                p2p
            ));
        }
    }

    let failures: Vec<&BenchmarkRecord> = records.iter().filter(|r| !r.success).collect();
    if !failures.is_empty() {
        report.push_str("\n## Failed Runs\n\n");
        for failure in failures {
            report.push_str(&format!(
                "- {} {} iteration {}: {}\n",
                failure.package,
                failure.mode,
                failure.iteration,
                failure.error.as_deref().unwrap_or("unknown error")
            ));
        }
    }

    std::fs::write(path, report)?;
    Ok(())
}

fn median_ms(mut values: Vec<u128>) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn csv_row(fields: &[String]) -> String {
    fields.iter().map(|field| csv_field(field)).collect::<Vec<_>>().join(",")
}

fn csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn rust_version() -> String {
    let Some(rustc) = find_on_path("rustc") else {
        return "unknown".to_string();
    };
    let Ok(output) = std::process::Command::new(rustc).arg("--version").output() else {
        return "unknown".to_string();
    };
    if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        "unknown".to_string()
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn canonicalize_existing(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    path.canonicalize().with_context(|| format!("failed to canonicalize {}", path.display()))
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("e2e crate has a parent workspace")
        .to_path_buf()
}

fn absolutize_path(root: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { root.join(path) }
}

fn reset_dir(path: &std::path::Path) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    std::fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_bool(name: &str) -> Option<bool> {
    match std::env::var(name).ok()?.as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn ensure_vm_prereqs(config: &VmConfig) -> anyhow::Result<()> {
    ensure_guix_p2p_binary(config)?;
    ensure_ssh_host_key(config)?;
    ensure_ssh_client_key(config)
}

fn ensure_guix_p2p_binary(config: &VmConfig) -> anyhow::Result<()> {
    if !config.guix_p2p_binary.is_file() {
        anyhow::bail!(
            "guix-p2p binary is missing or not executable; build it first with: guix shell -m \
             manifest.scm -- cargo build --release\nexpected binary: {}",
            config.guix_p2p_binary.display()
        );
    }
    Ok(())
}

fn ensure_ssh_host_key(config: &VmConfig) -> anyhow::Result<()> {
    if config.ssh_host_key.is_file() && config.ssh_host_key_pub.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(&config.ssh_dir)?;
    generate_ssh_key(&config.ssh_host_key, "guix-p2p-e2e")?;
    make_mode(&config.ssh_host_key, 0o600)?;
    make_mode(&config.ssh_host_key_pub, 0o644)
}

fn ensure_ssh_client_key(config: &VmConfig) -> anyhow::Result<()> {
    if config.ssh_client_key.is_file() && config.ssh_client_key_pub.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(&config.ssh_dir)?;
    generate_ssh_key(&config.ssh_client_key, "guix-p2p-e2e-client")?;
    make_mode(&config.ssh_client_key, 0o600)?;
    make_mode(&config.ssh_client_key_pub, 0o644)
}

fn generate_ssh_key(path: &std::path::Path, comment: &str) -> anyhow::Result<()> {
    let mut command = if let Some(ssh_keygen) = find_on_path("ssh-keygen") {
        std::process::Command::new(ssh_keygen)
    } else {
        let mut command = std::process::Command::new("guix");
        command.arg("shell").arg("openssh").arg("--").arg("ssh-keygen");
        command
    };
    command.args(["-t", "ed25519", "-N", "", "-C", comment, "-f"]).arg(path);
    checked_status(command, &format!("ssh-keygen {}", path.display()))
}

fn make_user_writable(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o200);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn make_mode(path: &std::path::Path, mode: u32) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn unique_slug(name: &str, registry: &VmRegistry) -> String {
    let base = slugify_node_name(name);
    if !registry.nodes.iter().any(|node| node.slug == base) {
        return base;
    }
    for idx in 2.. {
        let candidate = format!("{base}-{idx}");
        if !registry.nodes.iter().any(|node| node.slug == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn slugify_node_name(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "node".to_string() } else { slug }
}

fn tcp_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn is_pid_running(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    #[cfg(unix)]
    {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn stop_pid(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("kill").arg("-TERM").arg(pid.to_string()).status()?;
    if !status.success() && !is_pid_running(Some(pid)) {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while is_pid_running(Some(pid)) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if is_pid_running(Some(pid)) {
        let _ = std::process::Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
    }
    Ok(())
}

fn ssh_command(config: &VmConfig, node: &VmNode) -> std::process::Command {
    ssh_command_inner(config, node, false)
}

fn ssh_batch_command(config: &VmConfig, node: &VmNode) -> std::process::Command {
    ssh_command_inner(config, node, true)
}

fn ssh_command_inner(config: &VmConfig, node: &VmNode, batch: bool) -> std::process::Command {
    let mut command = std::process::Command::new("ssh");
    command
        .arg("-i")
        .arg(&config.ssh_client_key)
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", config.ssh_dir.join("known_hosts").display()))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-p")
        .arg(node.ssh_port.to_string());
    if batch {
        command.arg("-o").arg("BatchMode=yes");
    }
    command.arg("e2e@127.0.0.1");
    command
}

fn ssh_run(config: &VmConfig, node: &VmNode, remote: &str) -> anyhow::Result<String> {
    ensure_ssh_client_key(config)?;
    let output = ssh_batch_command(config, node)
        .arg(remote)
        .output()
        .with_context(|| format!("failed to run SSH command on {}", node.name))?;
    if !output.status.success() {
        anyhow::bail!(
            "SSH command on {} failed with {}\nstdout:\n{}\nstderr:\n{}",
            node.name,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn wait_ssh(config: &VmConfig, node: &VmNode) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if ssh_batch_command(config, node).arg("true").status().is_ok_and(|s| s.success()) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("{} SSH did not become ready on port {}", node.name, node.ssh_port);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn push_binary_to_node(
    config: &VmConfig,
    node: &VmNode,
    libgcrypt_runtime: &str,
) -> anyhow::Result<()> {
    ensure_guix_p2p_binary(config)?;
    ensure_ssh_client_key(config)?;
    let status = std::process::Command::new("scp")
        .arg("-i")
        .arg(&config.ssh_client_key)
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", config.ssh_dir.join("known_hosts").display()))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-P")
        .arg(node.ssh_port.to_string())
        .arg(&config.guix_p2p_binary)
        .arg("e2e@127.0.0.1:/tmp/guix-p2p-real")
        .status()?;
    if !status.success() {
        anyhow::bail!("scp to {} failed with {status}", node.name);
    }
    let install = format!(
        r#"set -eu
cat > /tmp/guix-p2p <<'EOF'
#!/bin/sh
set -eu
LIBGCRYPT="${{GUIX_P2P_E2E_LIBGCRYPT:-{}}}"
export LD_LIBRARY_PATH="$LIBGCRYPT/lib:/run/current-system/profile/lib${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
exec /tmp/guix-p2p-real "$@"
EOF
chmod 755 /tmp/guix-p2p /tmp/guix-p2p-real
"#,
        libgcrypt_runtime
    );
    ssh_run(config, node, &install)?;
    Ok(())
}

fn guix_build_last_path(package: &str) -> anyhow::Result<String> {
    let output = checked_output(
        std::process::Command::new("guix").arg("build").arg(package),
        &format!("guix build {package}"),
    )?;
    String::from_utf8(output.stdout)?
        .lines()
        .rev()
        .find(|line| line.starts_with("/gnu/store/"))
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("guix build {package} did not print a store path"))
}

fn parse_key_line(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    output.lines().filter_map(|line| line.strip_prefix(&prefix)).last().map(str::to_string)
}

fn write_vm_env(config: &VmConfig, node: &VmNode, seed: &VmSeed) -> anyhow::Result<()> {
    let content = format!(
        "export STORE_PATH={}\nexport PEER_ID={}\nexport E2E_PACKAGE={}\n",
        shell_quote(&seed.store_path),
        shell_quote(&seed.peer_id),
        shell_quote(&seed.package)
    );
    std::fs::write(config.proof_env_path(node), content)?;
    Ok(())
}

fn print_vm_env(seed: &VmSeed) {
    println!("export STORE_PATH={}", shell_quote(&seed.store_path));
    println!("export PEER_ID={}", shell_quote(&seed.peer_id));
    println!("export E2E_PACKAGE={}", shell_quote(&seed.package));
}

fn resolve_fetch_target(
    registry: &VmRegistry,
    node: &VmNode,
    explicit_store_path: Option<&str>,
    explicit_package: Option<&str>,
) -> anyhow::Result<VmFetch> {
    if let Some(existing) = &node.last_fetch {
        let mut target = existing.clone();
        if let Some(path) = explicit_store_path {
            target.store_path = path.to_string();
        }
        if let Some(package) = explicit_package {
            target.package = package.to_string();
        }
        return Ok(target);
    }

    let (seed_node, seed) = registry.latest_seed()?;
    Ok(VmFetch {
        from: seed_node.name.clone(),
        package: explicit_package.unwrap_or(&seed.package).to_string(),
        store_path: explicit_store_path.unwrap_or(&seed.store_path).to_string(),
        peer_id: seed.peer_id.clone(),
    })
}

fn vm_peer_multiaddr(node: &VmNode, peer_id: &str) -> String {
    format!("/ip4/10.0.2.2/tcp/{}/p2p/{peer_id}", node.p2p_port)
}

fn bootstrap_node_command() -> &'static str {
    r#"
set -eu
P2P="${GUIX_P2P_E2E_P2P_BIN:-guix-p2p}"
CACHE_DIR="${GUIX_P2P_E2E_BOOTSTRAP_CACHE:-/tmp/guix-p2p-bootstrap}"
LOG="${GUIX_P2P_E2E_BOOTSTRAP_LOG:-/tmp/guix-p2p-bootstrap.log}"
SOCKET="${GUIX_P2P_E2E_BOOTSTRAP_SOCKET:-$CACHE_DIR/guix-p2p.sock}"
LISTEN="${GUIX_P2P_E2E_BOOTSTRAP_LISTEN:-/ip4/0.0.0.0/tcp/6881}"
DASHBOARD_BIND="${GUIX_P2P_E2E_BOOTSTRAP_DASHBOARD_BIND:-0.0.0.0}"
DASHBOARD_PORT="${GUIX_P2P_E2E_BOOTSTRAP_DASHBOARD_PORT:-3031}"
mkdir -p "$CACHE_DIR" "$HOME/.config/guix-p2p"
printf 'min_providers = 1\n' > "$HOME/.config/guix-p2p/config.toml"
if [ -f /tmp/guix-p2p-bootstrap.pid ]; then
  OLD_PID="$(cat /tmp/guix-p2p-bootstrap.pid 2>/dev/null || true)"
  if [ -n "$OLD_PID" ] && kill -0 "$OLD_PID" 2>/dev/null; then
    kill "$OLD_PID" 2>/dev/null || true
    sleep 1
  fi
fi
rm -f "$SOCKET"
RUST_LOG="${RUST_LOG:-info}" "$P2P" --daemon \
  --cache-dir "$CACHE_DIR" \
  --listen-addr "$LISTEN" \
  --socket "$SOCKET" \
  --dashboard --dashboard-bind "$DASHBOARD_BIND" --dashboard-port "$DASHBOARD_PORT" \
  > "$LOG" 2>&1 &
PID="$!"
printf '%s\n' "$PID" > /tmp/guix-p2p-bootstrap.pid
PEER_ID=''
i=0
while [ "$i" -lt 30 ]; do
  PEER_ID="$(sed -n 's/.*Peer ID: //p' "$LOG" 2>/dev/null | tail -n 1)"
  [ -n "$PEER_ID" ] && break
  i=$((i + 1))
  sleep 1
done
[ -n "$PEER_ID" ] && printf '%s\n' "$PEER_ID" > /tmp/guix-p2p-bootstrap-peer-id
printf 'peer_id=%s\n' "$PEER_ID"
printf 'pid=%s\nlog=%s\nsocket=%s\ndashboard=http://127.0.0.1:%s\n' "$PID" "$LOG" "$SOCKET" "$DASHBOARD_PORT"
"#
}

fn store_hash_part(store_path: &str) -> Option<String> {
    let rest = store_path.strip_prefix("/gnu/store/")?;
    Some(rest.split_once('-')?.0.to_string())
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn json_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_string)
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn sanitize_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or("")
}

fn read_tail(path: &std::path::Path, max_lines: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn unix_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("{} seconds since 1970-01-01 UTC", duration.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.2} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugifies_arbitrary_node_names() {
        assert_eq!(slugify_node_name("Alice"), "alice");
        assert_eq!(slugify_node_name("Node 1"), "node-1");
        assert_eq!(slugify_node_name("  !@#  "), "node");
        assert_eq!(slugify_node_name("build.fetch"), "build-fetch");
    }

    #[test]
    fn parses_latest_key_line() {
        let output = "store_path=/gnu/store/old\npeer_id=one\nstore_path=/gnu/store/new\n";
        assert_eq!(parse_key_line(output, "store_path").as_deref(), Some("/gnu/store/new"));
        assert_eq!(parse_key_line(output, "peer_id").as_deref(), Some("one"));
        assert_eq!(parse_key_line(output, "missing"), None);
    }

    #[test]
    fn unique_slug_appends_suffix_for_collisions() {
        let registry = VmRegistry {
            default_bootstrap: None,
            nodes: vec![VmNode {
                name: "Alice".to_string(),
                slug: "alice".to_string(),
                ssh_port: 2221,
                dashboard_port: 3031,
                p2p_port: 6881,
                disk: PathBuf::from("alice.qcow2"),
                pid: None,
                last_seed: None,
                last_fetch: None,
            }],
        };
        assert_eq!(unique_slug("Alice", &registry), "alice-2");
    }

    #[test]
    fn fetch_target_prefers_explicit_then_saved_then_latest_seed() {
        let alice_seed = VmSeed {
            package: "hello".to_string(),
            store_path: "/gnu/store/example-hello".to_string(),
            peer_id: "12D3KooWalice".to_string(),
        };
        let mut node = VmNode {
            name: "Bob".to_string(),
            slug: "bob".to_string(),
            ssh_port: 2222,
            dashboard_port: 3032,
            p2p_port: 6882,
            disk: PathBuf::from("bob.qcow2"),
            pid: None,
            last_seed: None,
            last_fetch: None,
        };
        let registry = VmRegistry {
            default_bootstrap: None,
            nodes: vec![
                VmNode {
                    name: "Alice".to_string(),
                    slug: "alice".to_string(),
                    ssh_port: 2221,
                    dashboard_port: 3031,
                    p2p_port: 6881,
                    disk: PathBuf::from("alice.qcow2"),
                    pid: None,
                    last_seed: Some(alice_seed.clone()),
                    last_fetch: None,
                },
                node.clone(),
            ],
        };
        let target = resolve_fetch_target(&registry, &node, None, None).unwrap();
        assert_eq!(target.from, "Alice");
        assert_eq!(target.package, "hello");
        assert_eq!(target.store_path, alice_seed.store_path);

        node.last_fetch = Some(VmFetch {
            from: "Alice".to_string(),
            package: "git".to_string(),
            store_path: "/gnu/store/example-git".to_string(),
            peer_id: "12D3KooWexample".to_string(),
        });
        let target = resolve_fetch_target(&registry, &node, None, None).unwrap();
        assert_eq!(target.package, "git");
        let target = resolve_fetch_target(&registry, &node, None, Some("emacs")).unwrap();
        assert_eq!(target.package, "emacs");
    }

    #[test]
    fn bootstrap_multiaddr_uses_vm_host_forward() {
        let registry = VmRegistry {
            default_bootstrap: Some(VmBootstrap {
                node: "Bootstrap".to_string(),
                peer_id: "12D3KooWbootstrap".to_string(),
            }),
            nodes: vec![VmNode {
                name: "Bootstrap".to_string(),
                slug: "bootstrap".to_string(),
                ssh_port: 2221,
                dashboard_port: 3031,
                p2p_port: 6881,
                disk: PathBuf::from("bootstrap.qcow2"),
                pid: None,
                last_seed: None,
                last_fetch: None,
            }],
        };
        assert_eq!(
            registry.bootstrap_multiaddr().unwrap().as_deref(),
            Some("/ip4/10.0.2.2/tcp/6881/p2p/12D3KooWbootstrap")
        );
    }
}
