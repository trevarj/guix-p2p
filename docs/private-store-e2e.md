# Private-Store E2E

The strict proof requires Node A and Node B to have separate writable
`/gnu/store` directories. Shared-store Guix containers cannot prove that Node B
does not already have Node A's package.

This path uses qcow2 Guix System images because `guix system vm` shares the
host store, while `guix system image --image-type=qcow2` creates a disk image
with its own store.

## Current Milestone

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
scripts/e2e-private-store.sh derivation
```

Build the base image and copy it to writable Node A and Node B disks:

```sh
scripts/e2e-private-store.sh image
```

Print QEMU launch commands:

```sh
scripts/e2e-private-store.sh launch-a
scripts/e2e-private-store.sh launch-b
```

Or run each VM in its own terminal:

```sh
scripts/e2e-private-store.sh run-a
scripts/e2e-private-store.sh run-b
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
locally.

Rerunning either helper stops the previous helper-started daemon for that node
before starting a new one, avoiding stale dashboard or socket listeners.

For manual relay checks from a shell, connect fd 4 to stdout because the
Guix substitute protocol writes structured replies on fd 4:

```sh
printf 'have %s\n' "$STORE_PATH" \
  | guix-p2p --query --socket /tmp/guix-p2p-b/guix-p2p.sock 4>&1
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

- Prove both VMs boot and expose independent stores.
- Realize `hello` on Node A and prove Node B does not have it.
- Share the project payload into both VMs or bake it into the images.
- Start `guix-p2p` on both nodes with fixed TCP/dashboard ports.
- Start Node B's raw `guix-daemon` with the `GUIX` wrapper.
- Run `guix build hello` on Node B and require p2p-only substitution from Node A.
