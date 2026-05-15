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

Start four named nodes. Unknown names are auto-created from the base image and
assigned persistent ports:

```sh
cargo run -p guix-p2p-e2e -- vm run Bootstrap
cargo run -p guix-p2p-e2e -- vm run Alice
cargo run -p guix-p2p-e2e -- vm run Charles
cargo run -p guix-p2p-e2e -- vm run Bob
cargo run -p guix-p2p-e2e -- vm status --all
```

Run the proof:

```sh
cargo run -p guix-p2p-e2e -- vm proof
```

`vm proof` runs the documented Bootstrap/Alice/Charles/Bob sequence: wait for
SSH, push the release binary, start the bootstrap node, seed Alice and Charles,
remove the target from Bob, and fetch through Bob's p2p-only daemon. The proof
sets Bob's `max_in_flight_blocks_per_peer` to `1` so the small `hello` NAR must
download blocks from both seeders, then checks each seeder log for block-serving
evidence. Use this after large feature changes before trusting benchmark
results.

The steps are also available individually:

```sh
cargo run -p guix-p2p-e2e -- vm wait-ssh Bootstrap Alice Charles Bob
cargo run -p guix-p2p-e2e -- vm push-binary --all
cargo run -p guix-p2p-e2e -- vm bootstrap Bootstrap
cargo run -p guix-p2p-e2e -- vm seed Alice hello
cargo run -p guix-p2p-e2e -- vm seed Charles hello
cargo run -p guix-p2p-e2e -- vm remove Bob
cargo run -p guix-p2p-e2e -- vm fetch Bob
```

Run a VM benchmark after the same setup:

```sh
cargo run -p guix-p2p-e2e -- vm benchmark \
  --suite smoke \
  --modes http,p2p-only,p2p-first \
  --http-conditions normal \
  --seed-nodes Alice \
  --fetch-node Bob \
  --http-node Bob \
  --iterations 1
```

`bootstrap` starts a seedless DHT node and saves it as the default bootstrap
peer for the VM registry. `seed` uses that configured bootstrap peer, saves the
latest `store_path` and `peer_id` for the seed node, and announces the NAR
provider record through the DHT. Re-running `seed` updates the latest target
used by later `fetch` and `remove` commands.

`benchmark` assumes the same VM state is already configured: the base image
exists, the named VMs are running, SSH is ready, and a bootstrap node has been
saved with `vm bootstrap`. It seeds the requested
package on each `--seed-nodes` node, removes the target from the fetcher before
each fetch, runs HTTP-only fetches with the regular Guix daemon, and runs P2P
fetches through the same wrapper path as `vm fetch`.
VM commands use the `guix-p2p` binary embedded in the image by default. Use
`vm push-binary --all` plus `GUIX_P2P_E2E_P2P_BIN=/tmp/guix-p2p` only when you
need to test a replacement binary without rebuilding the image.
The VM wrapper launches Rust binaries through the VM profile's dynamic loader
so host-built Guix interpreter paths do not have to exist inside the guest.
Daemon readiness checks wait long enough for slow software-emulated runners and
print daemon log tails when a VM daemon exits or never reports its peer ID.

The VM harness passes each node's host-forwarded P2P port as an
`--external-addresses` value. This is required for QEMU user-mode networking:
the local guest address is not dialable from the other VMs, but the
host-forwarded address is.

`remove` realizes dependencies with the regular daemon path and deletes only
the target output. `fetch` starts the fetch node's P2P daemon against the
configured bootstrap peer and first verifies that the target is visible over
P2P. If no provider is found, it stops before starting the private
`guix-daemon` wrapper. Once the target is visible, `fetch` runs
`guix build --no-grafts` through the wrapper and requires the target output to
be imported as a store directory. Bob does not receive Alice's address directly
as a CLI argument in this flow; it learns the provider and its advertised
address from the DHT.

When dashboard forwarding is enabled, `fetch` also prints a
`DASHBOARD_EVIDENCE` block after the import proof. The block includes the
matching seed-node `/api/seeds` entry, the matching fetch-node `/api/catalog`
entry, and entry counts for both dashboard snapshots. If dashboard forwarding
is disabled, the command prints `DASHBOARD_EVIDENCE_SKIPPED`.

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
cargo run -p guix-p2p-e2e -- vm logs Bootstrap
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
- `nodes.json`: named node registry, ports, disks, PIDs, default bootstrap, and latest seed/fetch metadata
- `<node>.qcow2`: writable node disk
- `<node>.env`: shell exports for the node's latest seed
- `ssh/e2e_ed25519`: persistent test SSH client key
- `ssh/ssh_host_ed25519_key`: persistent test host key embedded in the image
- `logs/`: QEMU, serial, and image build logs

## Notes

- `guix-p2p-e2e dashboard-demo --bind 127.0.0.1 --port 3030` serves the real
  embedded dashboard with deterministic demo data. It is intended for demos,
  screenshots, and UI review without starting daemon nodes. The header marks
  the page as demo data.
- All nodes use the same image and can act as seeder or fetcher.
- If `fetch` reports `TARGET_NOT_AVAILABLE_OVER_P2P`, re-run `vm seed <node>
  hello` and inspect `vm logs <seed-node>` and `vm logs <fetch-node>`. The
  seed node must connect through the saved bootstrap peer before the fetch node
  can discover it.
- `run` uses KVM when available unless `--enable-kvm=false` or
  `GUIX_P2P_E2E_ENABLE_KVM=false` is set.
- Dashboard forwarding is enabled by default. Disable it with
  `--forward-dashboard=false` or `GUIX_P2P_E2E_FORWARD_DASHBOARD=false`.
  Each VM node gets a unique host-side dashboard port (3031, 3032, ...) mapped
  to guest port 3031 inside the VM. VM command output prints the host-forwarded
  dashboard URL.
- The VM login remains `e2e:e2e`.
