#!/bin/sh
set -eu

usage() {
    cat >&2 <<'EOF'
usage: scripts/bump-version.sh <major|minor|patch|x.y.z>

Updates:
  - Cargo.toml
  - Cargo.lock
  - channel/guix-p2p/packages.scm
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

[ "$#" -eq 1 ] || {
    usage
    exit 2
}

current=$(
    sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"/\1/p' Cargo.toml |
        sed -n '1p'
)

[ -n "$current" ] || die "could not read package version from Cargo.toml"

major=${current%%.*}
rest=${current#*.}
minor=${rest%%.*}
patch=${rest#*.}

case "$1" in
major)
    new_version="$((major + 1)).0.0"
    ;;
minor)
    new_version="$major.$((minor + 1)).0"
    ;;
patch)
    new_version="$major.$minor.$((patch + 1))"
    ;;
*[!0-9.]* | *.*.*.* | .* | *. | *..*)
    usage
    exit 2
    ;;
*)
    new_version="$1"
    ;;
esac

case "$new_version" in
[0-9]*.[0-9]*.[0-9]*) ;;
*)
    usage
    exit 2
    ;;
esac

old_cargo_lock=$(mktemp)
new_cargo_lock=$(mktemp)
trap 'rm -f "$old_cargo_lock" "$new_cargo_lock"' EXIT

cp Cargo.lock "$old_cargo_lock"

sed -i "0,/^version = \"$current\"/s//version = \"$new_version\"/" Cargo.toml
sed -i "s/^\([[:space:]]*(version \"\)[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\(\".*\)$/\1$new_version\2/" \
    channel/guix-p2p/packages.scm

awk -v new_version="$new_version" '
    $0 == "[[package]]" {
        in_package = 1
        package_name = ""
    }
    in_package && $0 ~ /^name = / {
        package_name = $0
    }
    in_package && package_name == "name = \"guix-p2p\"" && $0 ~ /^version = / {
        print "version = \"" new_version "\""
        in_package = 0
        next
    }
    { print }
' "$old_cargo_lock" >"$new_cargo_lock"
mv "$new_cargo_lock" Cargo.lock

cargo_version=$(
    sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"/\1/p' Cargo.toml |
        sed -n '1p'
)
lock_version=$(
    awk '
        $0 == "[[package]]" {
            in_package = 1
            package_name = ""
        }
        in_package && $0 ~ /^name = / {
            package_name = $0
        }
        in_package && package_name == "name = \"guix-p2p\"" && $0 ~ /^version = / {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' Cargo.lock
)
channel_version=$(
    sed -n 's/^[[:space:]]*(version "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)").*/\1/p' \
        channel/guix-p2p/packages.scm |
        sed -n '1p'
)

[ "$cargo_version" = "$new_version" ] || die "Cargo.toml version did not update"
[ "$lock_version" = "$new_version" ] || die "Cargo.lock version did not update"
[ "$channel_version" = "$new_version" ] || die "channel package version did not update"

printf 'bumped guix-p2p version: %s -> %s\n' "$current" "$new_version"
