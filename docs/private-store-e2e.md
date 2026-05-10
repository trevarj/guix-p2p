# Private-Store E2E

The strict proof requires Node A and Node B to have separate writable
`/gnu/store` directories. Shared-store Guix containers cannot prove that Node B
does not already have Node A's package.

This path uses qcow2 Guix System images because `guix system vm` shares the
host store, while `guix system image --image-type=qcow2` creates a disk image
with its own store.

## First Milestone

Node A and Node B are separate qcow2 images:

- Node A: includes `hello`.
- Node B: does not include `hello`.
- Both boot with serial console, DHCP, Guix daemon, and OpenSSH.

Build only derivations first:

```sh
scripts/e2e-private-store.sh derivation-a
scripts/e2e-private-store.sh derivation-b
```

Build writable disks:

```sh
scripts/e2e-private-store.sh image-a
scripts/e2e-private-store.sh image-b
```

Print QEMU launch commands:

```sh
scripts/e2e-private-store.sh launch-a
scripts/e2e-private-store.sh launch-b
```

The launch commands forward:

| Node | SSH | Dashboard | P2P TCP |
|------|-----|-----------|---------|
| A | `2221` | `3031` | `6881` |
| B | `2222` | `3032` | `6882` |

Serial logs are written under:

```sh
target/guix-p2p-private-store/logs/
```

## Next Milestones

- Prove both VMs boot and expose independent stores.
- Prove Node A has `hello` and Node B does not.
- Share the project payload into both VMs or bake it into the images.
- Start `guix-p2p` on both nodes with fixed TCP/dashboard ports.
- Start Node B's raw `guix-daemon` with the `GUIX` wrapper.
- Run `guix build hello` on Node B and require p2p-only substitution from Node A.
