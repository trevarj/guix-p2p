# Disposable E2E VM

Use this when the host exposes `/gnu/store` read-only. The real smoke proof
needs a raw `guix-daemon` to import substituted nars, so the test runs in a
throwaway Guix System qcow2 image with its own writable store.

By default this path downloads the official Guix System VM image if it is not
already cached under `target/guix-p2p-vm/`. For a quick dashboard demo, use:

```sh
scripts/e2e-fast-demo.sh
```

## Build and Boot

```sh
scripts/e2e-vm.sh run
```

This fetches the official Guix qcow2 image if needed, creates a small writable
qcow2 overlay at `target/guix-p2p-vm/e2e-vm.qcow2`, builds the binary payload
disk, and boots headlessly with QEMU on the serial console.

The writable qcow2 is disposable. If the Guix image symlink changes, the
script refreshes the writable disk before booting so stale GRUB or kernel
configuration is not reused.

The runner builds release binaries on the host, copies their shared library
dependencies into a small ext4 payload disk, and attaches that disk to QEMU as
a read-only virtio block device. The guest does not need the source checkout,
Cargo, or a QEMU shared filesystem.
The payload step patches the binaries' ELF interpreter and rpath to use
`/mnt/guix-p2p-bin/lib`, so the guest does not need the host's Rust toolchain
or library store paths.

Use `scripts/e2e-vm.sh image` to fetch the reusable official base image. Use
`scripts/e2e-vm.sh boot` to rebuild the binary payload disk and boot an
existing image, or `scripts/e2e-vm.sh run` to ensure both are present and boot.

The local service-baked image builder is still available with
`GUIX_P2P_E2E_IMAGE_SOURCE=local scripts/e2e-vm.sh image-local`, but it can
pull through a large Guix bootstrap closure. The official image path avoids
that local image build.

Dashboard ports are forwarded to the host:

- Node A: `http://127.0.0.1:3031`
- Node B: `http://127.0.0.1:3032`

The guest mounts the payload disk at `/mnt/guix-p2p-bin` and runs:

```sh
CARGO_TARGET_DIR=/tmp/guix-p2p-target \
  GUIX_P2P_E2E_BASE=/tmp/guix-p2p-e2e \
  GUIX_P2P_E2E_NO_GUIX_SHELL=1 \
  GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1 \
  LD_LIBRARY_PATH=/mnt/guix-p2p-bin/lib \
  /mnt/guix-p2p-bin/bin/guix-p2p-e2e container-smoke \
    --package /gnu/store/...-hello-....drv \
    --store-path /gnu/store/...-hello-... \
    --transport tcp \
    --guix-p2p-bin /mnt/guix-p2p-bin/bin/guix-p2p \
    --dashboard-bind 0.0.0.0 \
    --hold \
    --keep-temp
```

The payload also includes `/mnt/guix-p2p-bin/run-e2e-service.sh`, which mounts
the payload if needed and starts the smoke harness with the transferred
binaries. The official Guix image does not contain the project Shepherd service
ahead of time, so the next automation step is to trigger this runner inside the
guest after boot, preferably through SSH on the forwarded port `2222`.

Inside this disposable VM, `GUIX_P2P_E2E_NO_GUIX_SHELL=1` makes the harness
run the peer and daemon processes directly instead of nesting another `guix
shell -C`. The VM disk is already the isolation boundary, and this avoids
guest-side package bootstrapping before the smoke proof can start.

The local E2E peers use explicit loopback bootstrap addresses and disable mDNS,
so sandboxed hosts that reject multicast sends do not emit mDNS permission
errors during the proof.

The VM passes the raw derivation for the image-provided hello output as the build
target because the `guix` command inside the image can come from a different
channel revision than the image expression that provided the seed. After Node A
seeds the nar, `GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1` removes that store item
from the writable VM disk before Node B's isolated daemon runs, so the derivation
realization still has to substitute it.

## Why Not `guix system vm`

`guix system vm` shares the host store. If the host store is read-only, the VM
inherits the same blocker. The E2E runner uses `guix system image -t qcow2` and
boots a writable copy so the proof is isolated from the host.

## Expected Proof

The smoke command must pass all strict checks before it holds the dashboards:

- `guix build /gnu/store/...-hello-....drv` exits successfully through Node B's isolated daemon.
- Node A `/api/seeds` includes the seeded nar.
- Node B `/api/catalog` includes the requested store path or nar hash.
- Node A logs show block serving.
- Node B logs show p2p-only substitute handling and successful download.
- Node B logs do not show HTTP nar fallback.

Guest log:

```sh
/var/log/guix-p2p-e2e.log
```

Harness state and per-process logs:

```sh
/tmp/guix-p2p-e2e/logs/
```

## Runner Commands

```sh
scripts/e2e-vm.sh image
scripts/e2e-vm.sh image-local
scripts/e2e-vm.sh payload
scripts/e2e-vm.sh boot
scripts/e2e-vm.sh run
scripts/e2e-vm.sh clean
```

Environment overrides:

- `GUIX_P2P_E2E_BASE_IMAGE_URL`: Guix qcow2 URL, default Guix 1.5.0 x86_64.
- `GUIX_P2P_E2E_IMAGE_SOURCE`: `official` or `local`, default `official`.
- `GUIX_P2P_E2E_VM_SIZE`: image size, default `20G`.
- `GUIX_P2P_E2E_VM_MEMORY`: QEMU memory in MB, default `4096`.
- `GUIX_P2P_E2E_VM_CPUS`: QEMU CPU count, default `2`.
- `GUIX_P2P_E2E_PAYLOAD_SIZE_MB`: binary payload disk size, default `128`.
- `GUIX_P2P_E2E_IMAGE_REPAIR_ATTEMPTS`: missing store path retries, default `20`.

The runner uses KVM when `/dev/kvm` is available and falls back to slower TCG
otherwise.

## Troubleshooting

If QEMU stops at `Welcome to GRUB!`, rebuild and boot the serial-configured
image:

```sh
scripts/e2e-vm.sh run
```

If an old QEMU process still holds the qcow2 lock, stop that process and rerun
the command.

If the guest fails before the smoke proof starts, rebuild the image with
`scripts/e2e-vm.sh image`. If only the Rust code changed, `scripts/e2e-vm.sh
boot` is enough because it rebuilds the attached payload disk. Smoke-test
progress is mirrored to the serial console and to `/var/log/guix-p2p-e2e.log`
inside the guest.
