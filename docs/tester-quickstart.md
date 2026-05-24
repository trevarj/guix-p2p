# Tester Quickstart

This is the known-tester path for a Guix System machine. It assumes the node is
allowed to use normal Guix HTTP substitutes for narinfo metadata and fallback,
and uses `http-first` by default while the public P2P network is sparse.
Switch to `p2p-first` only when deliberately validating that a seeded package
can be fetched from a peer.

## 1. Add The Channel

Add `guix-p2p` to `~/.config/guix/channels.scm` or to the channels file you use
for `guix pull`:

```scheme
(cons*
 (channel
  (name 'guix-p2p)
  (url "https://codeberg.org/trevarj/guix-p2p")
  (branch "master")
  (introduction
   (make-channel-introduction
    "7d6c023e60cffde9cc63d7c4c2232a3f9c9fca1f"
    (openpgp-fingerprint
     "A6C2 0D0C 2AD8 38F9 4907  0EA3 A52D 6879 4EBE D758"))))
 %default-channels)
```

Then pull the authenticated channel:

```sh
guix pull
```

If Guix reports that the channel is not trusted, confirm that the
`introduction` block above is present and that your channel entry uses
`branch "master"`.

## 2. Enable The System Service

Import the service module in your `operating-system` configuration:

```scheme
(use-modules (guix-p2p services))
```

Add the daemon service and enable the substitute extension for `guix-daemon`:

```scheme
(services
  (modify-services
      (cons (service guix-p2p-service-type
                     (guix-p2p-configuration
                      (dashboard? #t)))
            %base-services)
    (guix-service-type config =>
      (guix-p2p-enable-guix-daemon-extension config))))
```

The service defaults to the project bootstrap node:

```text
/dns4/guix-p2p.trevs.site/tcp/443/p2p/12D3KooWDnvPgCuPTPaMbnbLpXP7kCxmXc9F7agJPuAJWXGoDNPT
```

You do not need to put this bootstrap peer in a user
`~/.config/guix-p2p/config.toml` for the system service. Shepherd starts the
daemon from the service configuration, so service fields are the source of
truth for daemon networking.

## 3. Reconfigure And Restart

Reconfigure the system and restart both long-running services:

```sh
sudo guix system reconfigure /etc/config.scm
sudo herd restart guix-p2p
sudo herd restart guix-daemon
```

Run this same restart sequence after future `guix pull` updates. The running
daemon and `guix-daemon` keep old store paths until restarted.

Confirm the installed binary and Git commit:

```sh
guix-p2p --version
```

## 4. Check Readiness

Run:

```sh
guix-p2p --doctor
```

Expected first-tester state:

- `guix-integration`: `ok`
- `daemon`: `ok`
- `bootstrap-peers`: `ok`
- `substitute-urls`: `ok`
- `shareable-address`: warning is acceptable for a laptop/client node

Open the dashboard:

```text
http://127.0.0.1:3030
```

Use the header readiness strip and the `node details` drawer to confirm:

- the version/commit is current
- bootstrap peers are configured
- the daemon socket is reachable
- peers eventually connect
- the event log updates while running Guix commands

For issue reports or automation:

```sh
guix-p2p --doctor --json
curl -s http://127.0.0.1:3030/api/status
curl -s http://127.0.0.1:3030/api/diagnostics
```

## 5. Seed A Package From One Node

On the seeding node, either use the dashboard package list and press `seed`, or
add a store path to the service configuration:

```scheme
(service guix-p2p-service-type
         (guix-p2p-configuration
          (dashboard? #t)
          (extra-options
           '("--seed" "/gnu/store/...-sl-5.05"))))
```

The dashboard `active seeds` panel should show the package with a readable
store-path label. Rows tagged `manual` came from the package list or explicit
seed paths. Rows tagged `auto` came from successful P2P downloads cached for
re-sharing. Rows tagged `cache` were found in the cache at daemon startup
without stored provenance; they may show only a NAR hash until the daemon
observes trusted narinfo/catalog metadata that identifies the store path.

## 6. Fetch From Another Node

On the fetching node, choose a package that is seeded by the other node and not
already in the local store. Temporarily set `(policy "p2p-first")` in the
fetching node service config when you want to force a P2P attempt before HTTP.
For a small test package:

```sh
guix build sl
```

A normal `http-first` fetch can complete from HTTP without touching P2P. A
successful P2P-preferred fetch prints a P2P attempt before any fallback. When
connected-provider fallback is used, the Guix output includes:

```text
downloading from p2p+connected-fallback://...
```

The dashboard should also show:

- a transfer path entry
- block movement events
- a successful import event
- an `auto` seed if `auto_seed_downloads` is `p2p` or `all`

## 7. If The Package Already Exists

First find the exact store path:

```sh
guix build sl
```

If Guix prints the path immediately, it is already present. Check why it is
alive:

```sh
guix gc --referrers /gnu/store/...-sl-5.05
```

Common referrers are profile generations, VM images, or system images. Do not
delete random profile or image roots unless you know they are disposable. For a
clean test, it is usually safer to pick another small package that is absent
from the fetcher.

If the only live root is a disposable profile generation you intentionally want
to remove, delete that generation through Guix profile commands, then collect
garbage:

```sh
guix package --list-generations
guix package --delete-generations=PATTERN
guix gc
```

Then verify the package path is gone before re-running the fetch test.

## 8. Connectivity Expectations

Client-only nodes can fetch from public or reachable peers without exposing
their own public IP. They may not be able to seed to other Internet peers
unless one of these is true:

- they advertise a reachable `external-addresses` value
- they are on the same LAN and mDNS works
- a future relay/Tor transport path is added

For now, the project bootstrap node helps peers find each other. It is not a
privacy relay and does not make a NATed laptop dialable from the public
Internet.

## 9. Useful Failure Report

Use [`tester-issue-template.md`](tester-issue-template.md) when filing a
tester failure.

Include:

- `guix-p2p --version`
- `guix-p2p --doctor`
- dashboard `copy diag`
- dashboard `copy peer` if this node should be dialable
- `guix-p2p --share-info` only when run with the same config and cache as the
  daemon
- `/api/status` and `/api/diagnostics` if the dashboard is reachable
- daemon command line and service config with private data removed
- the package name and store path being tested
- last 100 daemon log lines around the failure

Do not include secrets, private keys, or unrelated environment files.
