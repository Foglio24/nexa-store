#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
with_snap=false
for argument in "$@"; do
  case "$argument" in
    --with-snap) with_snap=true ;;
    -h|--help)
      printf 'Usage: %s [--with-snap]\n' "$0"
      printf 'Checks that the system is Debian, Ubuntu or a distribution based on them\n'
      printf '(Linux Mint, Pop!_OS, Zorin OS, elementary OS, MX Linux, Kali, Raspberry Pi OS, …)\n'
      printf 'and prepares the project dependencies.\n'
      printf 'Installing missing packages requires an interactive confirmation.\n'
      printf 'With --with-snap it also includes snapd, if it is not installed yet.\n'
      exit 0 ;;
    *) printf 'Unrecognized option: %s\nUse --help for instructions.\n' "$argument" >&2; exit 2 ;;
  esac
done

os_release=/etc/os-release
[[ -r "$os_release" ]] || os_release=/usr/lib/os-release
ID='' ID_LIKE='' PRETTY_NAME=''
if [[ -r "$os_release" ]]; then
  # System file provided by the distribution.
  source "$os_release"
fi
family=false
for name in ${ID:-} ${ID_LIKE:-}; do
  case "${name,,}" in debian|ubuntu) family=true ;; esac
done
# A derivative that does not declare its family is still fine when dpkg and APT manage it.
if ! "$family" && command -v apt-get >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1 && [[ -f /var/lib/dpkg/status ]]; then
  family=true
fi
if ! "$family"; then
  printf 'Nexa Store supports Debian, Ubuntu and the distributions based on them. Detected system: %s.\n' "${PRETTY_NAME:-unknown}" >&2
  exit 1
fi
printf 'System: %s\nProject: %s\n' "${PRETTY_NAME:-Debian-based Linux}" "$project_dir"

# The version APT would install for a package, empty when no repository offers it.
candidate() {
  LC_ALL=C apt-cache policy -- "$1" 2>/dev/null | awk '$1 == "Candidate:" { if ($2 != "(none)") print $2; exit }'
}
installed() {
  [[ "$(dpkg-query -W -f='${Status}' "$1" 2>/dev/null || true)" == 'install ok installed' ]]
}
# The first of several alternative package names that is installed or installable.
first_available() {
  local name
  for name in "$@"; do
    if installed "$name" || [[ -n "$(candidate "$name")" ]]; then printf '%s\n' "$name"; return 0; fi
  done
  return 1
}

packages=(
  build-essential pkg-config file libssl-dev libgtk-3-dev libxdo-dev librsvg2-dev flatpak
)
unavailable=()
# WebKitGTK 4.1 is in Debian 12, Ubuntu 22.04, Linux Mint 21 and later.
if [[ -n "$(candidate libwebkit2gtk-4.1-dev)" ]] || installed libwebkit2gtk-4.1-dev; then
  packages+=(libwebkit2gtk-4.1-dev)
else
  unavailable+=('libwebkit2gtk-4.1-dev (WebKitGTK 4.1: Debian 12, Ubuntu 22.04, Linux Mint 21 or later)')
fi
# pkexec has its own package since Debian 12 and Ubuntu 24.04; before, it came with policykit-1.
if pkexec_package="$(first_available pkexec policykit-1)"; then packages+=("$pkexec_package"); else unavailable+=('pkexec / policykit-1'); fi
if indicator="$(first_available libayatana-appindicator3-dev libappindicator3-dev)"; then packages+=("$indicator"); else unavailable+=('libayatana-appindicator3-dev'); fi
# Optional at run time: AppImages start directly with FUSE 2 (libfuse2t64 since Debian 13 and Ubuntu 24.04).
if fuse="$(first_available libfuse2t64 libfuse2)"; then packages+=("$fuse"); fi

if (( ${#unavailable[@]} > 0 )); then
  printf '\nThese build dependencies are not offered by the configured repositories:\n' >&2
  printf '  %s\n' "${unavailable[@]}" >&2
  printf 'Nexa Store (Tauri 2) needs a release from 2022 or later. Update the distribution and try again.\n' >&2
  exit 1
fi

toolchain_notes=()
# Rust: the distribution's compiler is used only when it is recent enough.
if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
  rust_candidate="$(candidate rustc)"
  if [[ -n "$rust_candidate" ]] && dpkg --compare-versions "${rust_candidate#*:}" ge 1.90; then
    packages+=(rustc cargo)
  else
    toolchain_notes+=("Rust >= 1.90 is not in the repositories (found: ${rust_candidate:-none}). Install it with rustup: https://rustup.rs/")
  fi
fi
# Node.js: likewise, 20 or later.
if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  node_candidate="$(candidate nodejs)"
  if [[ -n "$node_candidate" ]] && dpkg --compare-versions "${node_candidate#*:}" ge 20; then
    packages+=(nodejs npm)
  else
    toolchain_notes+=("Node.js >= 20 is not in the repositories (found: ${node_candidate:-none}). See https://nodejs.org/en/download")
  fi
fi
if "$with_snap"; then
  packages+=(snapd)
fi

missing=()
for package in "${packages[@]}"; do
  installed "$package" || missing+=("$package")
done

if (( ${#missing[@]} > 0 )); then
  printf '\nMissing system dependencies:\n'
  printf '  %s\n' "${missing[@]}"
  printf '\nThis will run apt-get install --no-upgrade for these packages.\n'
  printf 'APT will show dependencies, the required space and its own confirmation.\n'
  printf 'The script does not upgrade the system and does not add repositories.\n'
  if [[ ! -t 0 ]]; then
    printf 'Run this script in an interactive terminal to authorize the installation.\n' >&2
    exit 1
  fi
  read -r -p 'Authorize installing the missing dependencies? [y/N] ' answer
  case "$answer" in
    y|Y|yes|Yes|YES|s|S|si|Si|SI) ;;
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

# rustup installs into ~/.cargo/bin, which a fresh terminal may not have on PATH yet.
if [[ -d "$HOME/.cargo/bin" && ":$PATH:" != *":$HOME/.cargo/bin:"* ]]; then
  PATH="$HOME/.cargo/bin:$PATH"
fi

for command_name in node npm rustc cargo pkg-config flatpak pkexec; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'Command still missing: %s.\n' "$command_name" >&2
    if (( ${#toolchain_notes[@]} > 0 )); then printf '%s\n' "${toolchain_notes[@]}" >&2; fi
    printf 'The script never downloads or runs remote scripts automatically.\n' >&2
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
if [[ -z "$(flatpak remotes 2>/dev/null)" ]]; then
  printf 'No Flatpak remote is configured. To use Flathub, for example:\n'
  printf '  flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo\n'
fi
if ! command -v snap >/dev/null 2>&1; then
  printf 'Snap is not installed: that source stays off (the others work normally).\n'
  printf 'To request its installation with a confirmation: ./scripts/setup.sh --with-snap\n'
else
  printf 'The snap command is present; the app also checks that the snapd service is available.\n'
fi
