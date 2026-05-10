# Disposable E2E VM

Use this when the host exposes `/gnu/store` read-only. The strict smoke proof
needs a raw `guix-daemon` that can import substituted nars, so the test runs in
a throwaway Guix System qcow2 image with its own writable store.

For a quick dashboard demo without the real Guix VM path, use:

```sh
scripts/e2e-fast-demo.sh
```

## Build and Boot

```sh
scripts/e2e-vm.sh run
```

This ensures a pinned Guix 1.5 qcow2 image exists, creates a fresh disposable
writable overlay at `target/guix-p2p-vm/e2e-vm.qcow2`, builds static release
binaries for the payload directory, and boots headlessly with QEMU. Long image,
payload, and VM steps emit timestamped heartbeat lines every 30 seconds by
default, with command output captured under `target/guix-p2p-vm/logs/`.

The image contains only the base services needed for the proof: Guix daemon,
networking, serial console boot, and the `guix-p2p-e2e` Shepherd service. It
does not include a desktop environment or SSH service.

Image builds resolve a pinned Guix 1.5 profile with:

```sh
guix time-machine -q \
  --url=https://codeberg.org/guix/guix.git \
  --commit=7c0cd7e45b0240b842b4f3e767599501eac42ee1
```

The runner copies that profile's `bin/guix` under
`target/guix-p2p-vm/time-machine-guix/`, keeps its Guile load paths isolated to
that pinned profile, adds a small compatibility import for the build-status
reporter, then runs `guix system image -t qcow2`.

Normal Rust changes only rebuild the shared payload directory. Rebuild the image
only when `guix/e2e-vm.scm` changes, the pinned Guix revision changes, or Guix
image state needs refreshing.

```sh
scripts/e2e-vm.sh image          # ensure image exists
scripts/e2e-vm.sh rebuild-image  # force image rebuild
scripts/e2e-vm.sh payload        # rebuild static payload
scripts/e2e-vm.sh boot           # rebuild payload and boot existing image
scripts/e2e-vm.sh run            # ensure image, rebuild payload, boot, validate
scripts/e2e-vm.sh clean          # remove generated VM state
```

`run` and `boot` return success only when the guest prints the VM proof PASS
marker to the QEMU serial log. Set `GUIX_P2P_E2E_HOLD=1` to keep the VM and
dashboards running after validation for manual inspection.

## Payload

The runner builds `guix-p2p` and `guix-p2p-e2e` for
`x86_64-unknown-linux-musl` with nightly `-Z build-std` and requires static
binaries. It does not copy host shared libraries, patch ELF interpreters, or
expose the host `/gnu/store` to the guest.

QEMU shares the payload directory with the guest through 9p:

```sh
target/guix-p2p-vm/e2e-payload-root -> /mnt/guix-p2p-bin
```

The VM Shepherd service runs:

```sh
/mnt/guix-p2p-bin/run-e2e-service.sh
```

The payload runner mounts the 9p share if needed, remounts the disposable
guest `/gnu/store` writable, and starts:

```sh
GUIX_P2P_E2E_BASE=/tmp/guix-p2p-e2e \
  GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1 \
  /mnt/guix-p2p-bin/bin/guix-p2p-e2e container-smoke \
    --package /gnu/store/...-hello-....drv \
    --store-path /gnu/store/...-hello-... \
    --transport tcp \
    --guix-p2p-bin /mnt/guix-p2p-bin/bin/guix-p2p \
    --dashboard-bind 0.0.0.0 \
    --vm-direct \
    --keep-temp
```

Inside this VM, `--vm-direct` makes the harness run the peer and daemon
processes directly instead of nesting another `guix shell -C`. The VM disk is
already the isolation boundary. The older `GUIX_P2P_E2E_NO_GUIX_SHELL=1`
compatibility path still behaves the same way.

The VM passes the raw derivation for the image-provided hello output as the
build target. After Node A seeds the nar,
`GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A=1` removes that store item from the
writable VM disk before Node B's isolated daemon runs, so realization must
substitute it.

## Why Not `guix system vm`

`guix system vm` shares the host store. If the host store is read-only, the VM
inherits the same blocker. The E2E runner uses `guix system image -t qcow2` and
boots a writable copy so the proof is isolated from the host.

## Expected Proof

The smoke command must pass all strict checks before the guest prints
`GUIX_P2P_E2E_RESULT=PASS` and powers off:

- `guix build /gnu/store/...-hello-....drv` exits successfully through Node B's isolated daemon.
- Node A `/api/seeds` includes the seeded nar.
- Node B `/api/catalog` includes the requested store path or nar hash.
- Node A logs show block serving.
- Node B logs show p2p-only substitute handling and successful download.
- Node B logs do not show HTTP nar fallback.

Dashboard ports are forwarded to the host:

- Node A: `http://127.0.0.1:3031`
- Node B: `http://127.0.0.1:3032`

Dashboard hold mode is opt-in:

```sh
GUIX_P2P_E2E_HOLD=1 scripts/e2e-vm.sh run
```

Guest log:

```sh
/var/log/guix-p2p-e2e.log
```

Harness state and per-process logs:

```sh
/tmp/guix-p2p-e2e/logs/
```

## Environment Overrides

- `GUIX_P2P_E2E_VM_DIR`: generated VM state directory, default `target/guix-p2p-vm`.
- `GUIX_P2P_E2E_VM_SIZE`: image size, default `20G`.
- `GUIX_P2P_E2E_VM_MEMORY`: QEMU memory in MB, default `4096`.
- `GUIX_P2P_E2E_VM_CPUS`: QEMU CPU count, default `2`.
- `GUIX_P2P_E2E_VM_DISPLAY`: QEMU display backend, default `none`.
- `GUIX_P2P_E2E_GUIX_URL`: Guix channel URL for time-machine image builds,
  default `https://codeberg.org/guix/guix.git`.
- `GUIX_P2P_E2E_GUIX_COMMIT`: Guix commit for time-machine image builds,
  default `7c0cd7e45b0240b842b4f3e767599501eac42ee1`.
- `GUIX_P2P_E2E_HOLD`: keep dashboards running after success, default `0`.
- `GUIX_P2P_E2E_LOG_DIR`: shell log directory, default `target/guix-p2p-vm/logs`.
- `GUIX_P2P_E2E_HEARTBEAT_SECS`: long command heartbeat interval, default `30`.
- `GUIX_P2P_E2E_HEARTBEAT_TAIL_LINES`: output log lines copied into each
  heartbeat, default `12`. Set to `0` to disable output snippets.
- `GUIX_P2P_E2E_HEARTBEAT_PROCESS_SNAPSHOT`: include focused process snapshots
  in heartbeats, default `1`. Set to `0` to disable.

The runner uses KVM when `/dev/kvm` is available and falls back to slower TCG
otherwise.

## Logs

Host-side wrapper logs:

```sh
target/guix-p2p-vm/logs/e2e-vm.log
target/guix-p2p-vm/logs/image-build.log
target/guix-p2p-vm/logs/payload-build.log
target/guix-p2p-vm/logs/qemu-serial.log
```

Heartbeat lines in `e2e-vm.log` include recent output from the active command's
log and a focused process snapshot. This is useful when Guix is querying
substitutes or the image build is quiet for long stretches.

Guest-side logs:

```sh
/var/log/guix-p2p-e2e-runner.log
/var/log/guix-p2p-e2e.log
/tmp/guix-p2p-e2e/logs/
```

## Troubleshooting

If the payload step fails before Cargo starts, enter the project environment and
confirm `rust-src` and `musl-gcc` are available:

```sh
guix shell -m manifest.scm -- sh -c \
  'test -d "$(rustc --print sysroot)/lib/rustlib/src/rust/library" && command -v musl-gcc'
```

If QEMU stops at `Welcome to GRUB!`, rebuild and boot the serial-configured
image:

```sh
scripts/e2e-vm.sh rebuild-image
scripts/e2e-vm.sh run
```

If an old QEMU process still holds the qcow2 lock, stop that process and rerun
the command.

If the pinned `guix system image` step reports that `/gnu/store/... is not
valid`, treat that as a host Guix store integrity issue. The VM runner stops
immediately and does not auto-repair host store paths. Repair the exact path or
run a broader store verification before retrying:

```sh
sudo guix build --repair /gnu/store/...-source.tar.xz
sudo guix gc --verify=contents,repair
scripts/e2e-vm.sh rebuild-image
```

Source tarballs such as `gzip-1.14.tar.xz` can appear here because the VM image
contains the transitive build/source closure for the seeded `hello` derivation.

If the guest fails before the smoke proof starts, rebuild the image with
`scripts/e2e-vm.sh rebuild-image`. If only Rust code changed,
`scripts/e2e-vm.sh boot` is enough because it rebuilds the shared payload.
Smoke-test progress is mirrored to the serial console and to
`/var/log/guix-p2p-e2e.log` inside the guest.
