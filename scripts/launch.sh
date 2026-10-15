#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
binary="$project_dir/src-tauri/target/release/nexa-store"

if [[ ! -x "$binary" ]]; then
  printf 'Nexa Store has not been built in release mode yet.\n' >&2
  printf 'Open a terminal in: %s\n' "$project_dir" >&2
  printf 'Build with: npm run package\n' >&2
  printf 'For development you can use: npm run desktop\n' >&2
  exit 1
fi

cd -- "$project_dir"
exec "$binary" "$@"
