#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo build --locked --release
exec "./target/release/leetdesk" "$@"
