#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
with_snap=false
for argument in "$@"; do
  case "$argument" in
    --with-snap) with_snap=true ;;
    -h|--help)
      printf 'Usage: %s [--with-snap]\n' "$0"
      printf 'Checks Ubuntu and prepares the project dependencies.\n'
      printf 'Installing missing packages requires an interactive confirmation.\n'
      printf 'With --with-snap it also includes snapd, if it is not installed yet.\n'
      exit 0 ;;
    *) printf 'Unrecognized option: %s\nUse --help for instructions.\n' "$argument" >&2; exit 2 ;;
  esac
done

if [[ ! -r /etc/os-release ]]; then
  printf 'Cannot identify Ubuntu: /etc/os-release is missing.\n' >&2
  exit 1
fi
# System file provided by the distribution.
source /etc/os-release
if [[ "${ID:-}" != ubuntu ]]; then
  printf 'Nexa Store supports Ubuntu only. Detected system: %s.\n' "${PRETTY_NAME:-unknown}" >&2
  exit 1
fi
printf 'System: %s\nProject: %s\n' "${PRETTY_NAME:-Ubuntu}" "$project_dir"

packages=(
  build-essential pkg-config file libssl-dev libwebkit2gtk-4.1-dev
  libgtk-3-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev
  flatpak pkexec
)
command -v rustc >/dev/null 2>&1 || packages+=(rustc)
command -v cargo >/dev/null 2>&1 || packages+=(cargo)
command -v node >/dev/null 2>&1 || packages+=(nodejs)
command -v npm >/dev/null 2>&1 || packages+=(npm)
if "$with_snap"; then
  packages+=(snapd)
fi

missing=()
for package in "${packages[@]}"; do
  if [[ "$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null || true)" != 'install ok installed' ]]; then
    missing+=("$package")
  fi
done

if (( ${#missing[@]} > 0 )); then
  printf '\nMissing system dependencies:\n'
  printf '  %s\n' "${missing[@]}"
  printf '\nThis will run apt-get install --no-upgrade for these packages.\n'
  printf 'APT will show dependencies, the required space and its own confirmation.\n'
  printf 'The script does not upgrade Ubuntu and does not add repositories.\n'
  if [[ ! -t 0 ]]; then
    printf 'Run this script in an interactive terminal to authorize the installation.\n' >&2
    exit 1
  fi
  read -r -p 'Authorize installing the missing dependencies? [y/N] ' answer
  case "$answer" in
    y|Y|yes|Yes|YES) ;;
    *) printf 'Installation cancelled. No package was changed.\n'; exit 1 ;;
  esac
  if (( EUID == 0 )); then
    apt-get install --no-upgrade "${missing[@]}"
  elif command -v sudo >/dev/null 2>&1; then
    sudo apt-get install --no-upgrade "${missing[@]}"
  else
    printf 'sudo is not available. Install the listed packages as an administrator and run the script again.\n' >&2
    exit 1
  fi
fi

for command_name in node npm rustc cargo pkg-config flatpak pkexec; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'Command still missing: %s. Check PATH and the installation.\n' "$command_name" >&2
    exit 1
  fi
done

node_major="$(node -p 'Number(process.versions.node.split(".")[0])')"
if (( node_major < 20 )); then
  printf 'Node.js 20 or later is required; found %s. Node.js 22/24 LTS is recommended.\n' "$(node --version)" >&2
  printf 'See https://nodejs.org/en/download and run the script again after installing it.\n' >&2
  exit 1
fi
rust_version="$(rustc --version | awk '{print $2}')"
if ! dpkg --compare-versions "$rust_version" ge 1.90; then
  printf 'Rust >= 1.90 is required; found %s.\n' "$rust_version" >&2
  printf 'See https://rustup.rs/ and install a stable toolchain following the official instructions.\n' >&2
  printf 'The script never downloads or runs remote scripts automatically.\n' >&2
  exit 1
fi

pkg-config --exists gtk+-3.0 webkit2gtk-4.1
printf '\nNode %s — npm %s — Rust %s\n' "$(node --version)" "$(npm --version)" "$rust_version"
printf 'Installing the JavaScript dependencies in the project…\n'
cd -- "$project_dir"
if [[ -f package-lock.json ]]; then
  npm ci
else
  npm install
fi
printf '\nEnvironment ready. Development: npm run desktop\n'
printf 'Installable package: npm run package\n'
printf 'Existing Flatpak remotes are left unchanged.\n'
if ! command -v snap >/dev/null 2>&1; then
  printf 'Snap is not available: that source will stay disabled.\n'
  printf 'To request its installation with a confirmation: ./scripts/setup-ubuntu.sh --with-snap\n'
else
  printf 'The snap command is present; the app also checks that the snapd service is available.\n'
fi
