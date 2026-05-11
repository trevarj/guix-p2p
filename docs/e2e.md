# Private-Store E2E

The strict proof requires Node A and Node B to have separate writable
`/gnu/store` directories. Shared-store Guix containers cannot prove that Node B
does not already have Node A's package.

This path uses qcow2 Guix System images because `guix system vm` shares the
host store, while `guix system image --image-type=qcow2` creates a disk image
with its own store.

Use `scripts/e2e.sh` for this workflow. The old
`scripts/e2e-private-store.sh` path is a compatibility wrapper.

## Current Milestone

Current status: the full raw `guix-daemon` proof is passing. Node B can query
Node A for the `hello` NAR, download it through `guix-p2p --substitute
--socket`, and complete `guix build --no-grafts hello` with the NAR imported
into Node B's private store.

Node A and Node B use separate writable qcow2 disks copied from one common
base image:

- The base image does not include `hello`.
- Both nodes boot with serial console, DHCP, Guix daemon, OpenSSH, and the
  locally built `guix-p2p` binary in the system profile.
- Node A will realize the package under test after boot.
- Node B starts from the same base image and must obtain the package through
  the private-store test flow.

Build the binary that will be embedded in the image:

```sh
cargo build --release
```

Build only the derivation first:

```sh
scripts/e2e.sh derivation
```

Build the base image and copy it to writable Node A and Node B disks:

```sh
scripts/e2e.sh image
```

To rerun the proof from clean VM disks without rebuilding the base image:

```sh
scripts/e2e.sh reset-disks
```

Print QEMU launch commands:

```sh
scripts/e2e.sh launch-a
scripts/e2e.sh launch-b
```

Or run each VM in its own terminal:

```sh
scripts/e2e.sh run-a
scripts/e2e.sh run-b
```

From a third terminal, wait for SSH, push the current binary, and start the two
node helpers:

```sh
scripts/e2e.sh wait-ssh
scripts/e2e.sh push-binary
scripts/e2e.sh node-a hello
scripts/e2e.sh node-b
```

`node-a` prints `store_path=...`, `peer_id=...`, and shell export lines. It
also saves them to `target/guix-p2p-private-store/e2e.env`, so later shortcut
commands can use them without extra arguments. To load the values into the
current shell anyway:

```sh
eval "$(scripts/e2e.sh env)"
```

Then run the full raw daemon proof:

```sh
scripts/e2e.sh prewarm-b
scripts/e2e.sh daemon-b
scripts/e2e.sh prove-b
```

Useful log tails:

```sh
scripts/e2e.sh logs-a
scripts/e2e.sh logs-b
scripts/e2e.sh daemon-log-b
```

After the first rebuild with the persistent test client key, SSH does not need
the password:

```sh
scripts/e2e.sh ssh-a
scripts/e2e.sh ssh-b
```

To iterate on Rust changes without rebuilding the image, rebuild the host
binary and copy it into both running VMs:

```sh
guix shell -m manifest.scm -- cargo build --release
scripts/e2e.sh push-binary
```

The push step installs `/tmp/guix-p2p-real` plus a `/tmp/guix-p2p` wrapper
that points at runtime libraries from the host-built closure and the VM system
profile.

Then run the VM helpers with the pushed binary:

```sh
GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p guix-p2p-e2e-node-a hello
GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p guix-p2p-e2e-node-b /gnu/store/... 12D3...
```

The launch commands forward:

| Node | SSH | Dashboard | P2P TCP |
|------|-----|-----------|---------|
| A | `2221` | `3031` | `6881` |
| B | `2222` | `3032` | `6882` |

Both disks include a test login:

```text
user: e2e
password: e2e
```

The image also authorizes a persistent test SSH client key generated under:

```sh
target/guix-p2p-private-store/ssh/e2e_ed25519
```

Inside either VM, verify the embedded binary:

```sh
command -v guix-p2p
guix-p2p --help
```

The image also includes VM-local setup helpers:

```sh
command -v guix-p2p-e2e-node-a
command -v guix-p2p-e2e-node-b
```

On Node A, build and seed the package under test:

```sh
guix-p2p-e2e-node-a hello
```

The Node A helper builds with `--no-grafts` and the configured substitute URLs,
then verifies that one of those URLs serves signed narinfo for the selected
store path. `--no-grafts` keeps the test on the substitutable output instead of
a locally grafted output that may not have public narinfo. The command prints
`store_path=...`, `narinfo_url=...`, and `peer_id=...`; pass the store path and
peer ID to Node B:

```sh
guix-p2p-e2e-node-b /gnu/store/...-hello-... 12D3...
```

The Node B helper refuses to continue if that exact store path already exists
locally. For the full `guix-daemon` proof, prewarm Node B with the regular
daemon and then delete only the target output so dependencies are present while
the package under test is absent:

```sh
guix build --no-grafts hello
guix gc -D /gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
```

Rerunning either helper stops the previous helper-started daemon for that node
before starting a new one, avoiding stale dashboard or socket listeners.

For manual relay checks from a shell, connect fd 4 to stdout because the
Guix substitute protocol writes structured replies on fd 4:

```sh
printf 'have %s\n' "$STORE_PATH" \
  | guix-p2p --query --socket /tmp/guix-p2p-b/guix-p2p.sock 4>&1
```

To test the P2P substitute path without running `guix-daemon`, request a
single-item destination on Node B:

```sh
rm -rf /tmp/guix-p2p-substitute
printf 'substitute %s /tmp/guix-p2p-substitute\n' "$STORE_PATH" \
  | guix-p2p --substitute --socket /tmp/guix-p2p-b/guix-p2p.sock 4>&1
test -d /tmp/guix-p2p-substitute
guix hash -f hex -r /tmp/guix-p2p-substitute
```

The expected success line is:

```text
success sha256:<nar-hash> <nar-size>
```

For the direct relay `hello` proof, the observed successful output was:

```text
success sha256:d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62 282616
```

For the full raw `guix-daemon` proof on Node B, the target was absent before
the build, then `GUIX_DAEMON_SOCKET=/tmp/e2e-guix-daemon.sock guix build
--no-grafts hello` returned and the imported store path was a directory:

```text
/gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
IMPORTED_HELLO_IN_NODE_B_STORE
```

`prove-b` intentionally checks `test -d "$STORE_PATH"` after the build. A raw
NAR file at the store path is not a valid substitute result and will fail later
when Guix tries to open the output as a directory.

Node B's `guix-p2p` log showed:

```text
Found 1 P2P providers for d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62
Handshake with 12D3KooWFWpapCwJbvM7m11YVMjp452oZzJKEhc6sC2jz46fjERt: 2 blocks available
Substitute download succeeded for /gnu/store/cs56i9digj9qg1bd383cmxc6xrfpdn9n-hello-2.12.2
```

Node A's log showed it served the NAR:

```text
serving 2 block(s): hash=d4d3119688670b12..
```

Serial logs are written under:

```sh
target/guix-p2p-private-store/logs/
```

The `launch-*` steps only print commands. The `run-*` steps actually start
QEMU in the foreground and create the serial log file.

The image embeds a persistent test-only OpenSSH host key from:

```sh
target/guix-p2p-private-store/ssh/ssh_host_ed25519_key
```

Rebuilding the image keeps the same SSH fingerprint. The first rebuild after
introducing this persistent key may still require removing the old generated
host key from `known_hosts` once:

```sh
ssh-keygen -R '[127.0.0.1]:2221'
ssh-keygen -R '[127.0.0.1]:2222'
```

## Next Milestones

- Prove both VMs boot and expose independent stores. Done.
- Realize `hello` on Node A and prove Node B does not have it. Done.
- Share the project payload into both VMs or bake it into the images. Done.
- Start `guix-p2p` on both nodes with fixed TCP/dashboard ports. Done.
- Verify direct P2P substitute relay from Node A to Node B. Done.
- Start Node B's raw `guix-daemon` with the `GUIX` wrapper. Done.
- Run `guix build hello` on Node B and require p2p-only substitution from Node A. Done.
