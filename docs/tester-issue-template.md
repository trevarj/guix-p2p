# Tester Issue Template

## Summary

- What failed:
- Expected result:
- Actual result:

## Environment

- `guix-p2p` version or commit:
- Guix System or Guix on another distro:
- Network type: LAN, public IPv4, IPv6, NAT/port-forwarded, VPN, or other:
- Transport: QUIC/UDP or TCP:

## Commands

```sh
guix-p2p --doctor
guix-p2p --share-info
guix-p2p --test-connectivity <peer-multiaddr>
```

Daemon command line:

```sh

```

Failed store path:

```text

```

## Dashboard Output

Attach these when the dashboard is reachable:

- `copy diag`
- `copy peer` when this node should be dialable
- Dashboard `NET` state
- Relevant events showing dial attempts, dial failures, inbound failures, or
  disconnect reasons

## Logs

Attach the last 100 daemon log lines around the failure:

```text

```

## Redactions

Remove secrets, private keys, tokens, unrelated environment variables, and
private hostnames that should not be public.
