#!/bin/sh
# Compatibility test entry point for the Rust wrapper routing tests.

set -eu

cd "$(dirname "$0")/.."
exec cargo test wrapper::tests
