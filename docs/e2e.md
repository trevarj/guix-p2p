# E2E VM Proof

The strict proof uses disposable qcow2 Guix System VMs so each node has its
own writable `/gnu/store`. Shared-store containers cannot prove that a fetcher
does not already have the package seeded by another node.

Enter the project shell first:

```sh
guix shell -m manifest.scm
```

Build the binaries:

```sh
cargo build --release
```

Build the base image:

```sh
cargo run -p guix-p2p-e2e -- vm image
```

Start two named nodes. Unknown names are auto-created from the base image and
assigned persistent ports:

```sh
cargo run -p guix-p2p-e2e -- vm run Alice
cargo run -p guix-p2p-e2e -- vm run Bob
cargo run -p guix-p2p-e2e -- vm status --all
```

Run the proof:

```sh
cargo run -p guix-p2p-e2e -- vm wait-ssh Alice Bob
cargo run -p guix-p2p-e2e -- vm push-binary --all
cargo run -p guix-p2p-e2e -- vm seed Alice hello
cargo run -p guix-p2p-e2e -- vm fetch Bob --from Alice
cargo run -p guix-p2p-e2e -- vm prewarm Bob
cargo run -p guix-p2p-e2e -- vm daemon Bob
cargo run -p guix-p2p-e2e -- vm prove Bob
```

`seed` saves the latest `store_path` and `peer_id` for that node. `fetch`
uses the latest seed from `--from`, starts the fetcher daemon, and records the
target package for later `prewarm`, `daemon`, and `prove` commands.

Verify the imported output by SSHing into the fetcher and running the store
path directly. The proof imports the output; it does not install `hello` into
the shell profile.

```sh
cargo run -p guix-p2p-e2e -- vm env Alice
cargo run -p guix-p2p-e2e -- vm ssh Bob
```

Useful operations:

```sh
cargo run -p guix-p2p-e2e -- vm logs Alice
cargo run -p guix-p2p-e2e -- vm logs Bob
cargo run -p guix-p2p-e2e -- vm daemon-log Bob
cargo run -p guix-p2p-e2e -- vm stop --all
```

## State

State defaults to:

```sh
target/guix-p2p-e2e/
```

Important files:

- `base.qcow2`: shared base image copied to node disks
- `nodes.json`: named node registry, ports, disks, PIDs, and latest seed/fetch metadata
- `<node>.qcow2`: writable node disk
- `<node>.env`: shell exports for the node's latest seed
- `ssh/e2e_ed25519`: persistent test SSH client key
- `ssh/ssh_host_ed25519_key`: persistent test host key embedded in the image
- `logs/`: QEMU, serial, and image build logs

## Notes

- All nodes use the same image and can act as seeder or fetcher.
- `run` uses KVM when available unless `--enable-kvm=false` or
  `GUIX_P2P_E2E_ENABLE_KVM=false` is set.
- Dashboard forwarding is enabled by default. Disable it with
  `--forward-dashboard=false` or `GUIX_P2P_E2E_FORWARD_DASHBOARD=false`.
- The VM login remains `e2e:e2e`.
