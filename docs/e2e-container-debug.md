# E2E Container Debug Path

Use this path to diagnose Guix container failures one layer at a time. It uses
plain `guix system container` commands and small Scheme operating-system files
under `guix/`; it does not use the E2E shell scripts.

Use official substitutes while debugging:

```sh
SUBS='https://ci.guix.gnu.org https://bordeaux.guix.gnu.org'
```

## Minimal Container

Prove the host build path first:

```sh
guix build hello --no-grafts --substitute-urls="$SUBS"
```

Build the minimal container script:

```sh
RUN_CONTAINER=$(guix system container -N \
  --substitute-urls="$SUBS" \
  guix/e2e-container-minimal.scm)
printf '%s\n' "$RUN_CONTAINER"
```

Launch it in one terminal:

```sh
sudo "$RUN_CONTAINER"
```

Enter it from another terminal with the printed PID:

```sh
sudo guix container exec PID \
  /run/current-system/profile/bin/bash --login
```

Inside the container:

```sh
hostname
herd status
guix --version
guix build hello --no-grafts --substitute-urls='https://ci.guix.gnu.org https://bordeaux.guix.gnu.org'
```

## Pinned Guix

Repeat the minimal container through the pinned Guix 1.5 time-machine revision:

```sh
RUN_CONTAINER_15=$(guix time-machine -q \
  --url=https://codeberg.org/guix/guix.git \
  --commit=7c0cd7e45b0240b842b4f3e767599501eac42ee1 \
  -- system container -N \
     --substitute-urls="$SUBS" \
     guix/e2e-container-minimal.scm)

sudo "$RUN_CONTAINER_15"
```

If this fails with `make-custom-binary-output-port: unbound variable`, stop
there. That failure happens while invoking the pinned Guix command and is not a
container launch failure. Continue the host-Guix steps first, then decide
whether to reuse the pinned-wrapper compatibility path from the VM runner.

## Trivial Shepherd Service

Build and launch a container that only adds a marker service:

```sh
RUN_SERVICE=$(guix system container -N \
  --substitute-urls="$SUBS" \
  guix/e2e-container-service.scm)

sudo "$RUN_SERVICE"
```

Enter it and verify:

```sh
herd status guix-p2p-debug
cat /var/log/guix-p2p-debug-service.log
```

## Project Daemon Nodes

Build the project binary first:

```sh
guix shell -m manifest.scm -- cargo build --release
```

Launch Node A with the checkout mounted at `/src`:

```sh
RUN_A=$(guix system container -N \
  --substitute-urls="$SUBS" \
  guix/e2e-container-node-a.scm)

sudo "$RUN_A" --share="$PWD=/src"
```

Verify from the host:

```sh
curl http://127.0.0.1:3031/api/status
```

Launch Node B:

```sh
RUN_B=$(guix system container -N \
  --substitute-urls="$SUBS" \
  guix/e2e-container-node-b.scm)

sudo "$RUN_B" --share="$PWD=/src"
```

Verify from the host:

```sh
curl http://127.0.0.1:3032/api/status
```

To test explicit bootstrap later, preserve `GUIX_P2P_BOOTSTRAP_PEERS` when
launching Node B:

```sh
sudo env GUIX_P2P_BOOTSTRAP_PEERS="$BOOTSTRAP" \
  "$RUN_B" --share="$PWD=/src"
```

## Failure Boundaries

- Host `guix build hello` fails: host store or substitutes are broken.
- Minimal container build fails: OS declaration or system container derivation.
- Minimal container launch fails: root/container namespace setup.
- Inside-container `herd` or `guix build hello` fails: runtime container,
  Shepherd, or Guix-in-container behavior.
- Pinned Guix fails after host Guix works: time-machine revision issue.
- Node containers fail after trivial service works: project binary or service
  integration issue.
