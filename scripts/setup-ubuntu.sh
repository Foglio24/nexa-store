#!/usr/bin/env bash
# Kept for existing instructions: the setup now covers every Debian or Ubuntu based system.
exec "$(dirname -- "${BASH_SOURCE[0]}")/setup.sh" "$@"
