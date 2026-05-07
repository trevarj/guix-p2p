# Disposable E2E VM

Use this when the host exposes `/gnu/store` read-only. The real smoke proof
needs a raw `guix-daemon` to import substituted nars, so the test runs in a
throwaway Guix System qcow2 image with its own writable store.

This is the slow strict-proof path. The first image build can download
`linux-libre`. For a quick dashboard demo, use:

```sh
scripts/e2e-fast-demo.sh
```

## Build and Boot

```sh
scripts/e2e-vm.sh run
```

This builds `guix/e2e-vm.scm`, copies the image to
`target/guix-p2p-vm/e2e-vm.qcow2`, and boots it with QEMU.

Dashboard ports are forwarded to the host:

- Node A: `http://127.0.0.1:3031`
- Node B: `http://127.0.0.1:3032`

The guest runs:

```sh
CARGO_TARGET_DIR=/tmp/guix-p2p-target \
  guix shell -m manifest.scm -- \
  cargo run -p guix-p2p-e2e -- container-smoke \
    --package hello \
    --transport tcp \
    --dashboard-bind 0.0.0.0 \
    --hold \
    --keep-temp
```

## Why Not `guix system vm`

`guix system vm` shares the host store. If the host store is read-only, the VM
inherits the same blocker. The E2E runner uses `guix system image -t qcow2` and
boots a writable copy so the proof is isolated from the host.

## Expected Proof

The smoke command must pass all strict checks before it holds the dashboards:

- `guix build hello` exits successfully through Node B's isolated daemon.
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
scripts/e2e-vm.sh boot
scripts/e2e-vm.sh run
scripts/e2e-vm.sh clean
```

Environment overrides:

- `GUIX_P2P_E2E_VM_SIZE`: image size, default `20G`.
- `GUIX_P2P_E2E_VM_MEMORY`: QEMU memory in MB, default `4096`.
- `GUIX_P2P_E2E_VM_CPUS`: QEMU CPU count, default `2`.

The runner uses KVM when `/dev/kvm` is available and falls back to slower TCG
otherwise.
