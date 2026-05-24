#!/usr/bin/env bash
set -eu

branch="${1:-master}"
codeberg_remote="${CODEBERG_REMOTE:-https://codeberg.org/trevarj/guix-p2p.git}"
github_remote="${GITHUB_REMOTE:-https://github.com/trevarj/guix-p2p.git}"
timeout_secs="${MIRROR_CHECK_TIMEOUT_SECS:-15}"

remote_head() {
  timeout "$timeout_secs" git ls-remote "$1" "refs/heads/$branch" | awk '{ print $1 }'
}

local_head="$(git rev-parse HEAD)"
codeberg_head="$(remote_head "$codeberg_remote")"
github_head="$(remote_head "$github_remote")"

printf 'branch: %s\n' "$branch"
printf 'local:   %s\n' "$local_head"
printf 'codeberg:%s\n' "${codeberg_head:+ $codeberg_head}"
printf 'github:  %s\n' "$github_head"

if [ -z "$codeberg_head" ]; then
  printf 'error: could not read Codeberg remote %s\n' "$codeberg_remote" >&2
  exit 2
fi

if [ -z "$github_head" ]; then
  printf 'error: could not read GitHub remote %s\n' "$github_remote" >&2
  exit 2
fi

if [ "$codeberg_head" != "$github_head" ]; then
  printf 'error: GitHub mirror is stale or divergent\n' >&2
  printf 'hint: push/sync Codeberg to GitHub before expecting Pages to update\n' >&2
  exit 1
fi

if [ "$local_head" != "$codeberg_head" ]; then
  printf 'warn: local HEAD differs from Codeberg %s\n' "$branch" >&2
fi

printf 'ok: GitHub mirror matches Codeberg\n'
