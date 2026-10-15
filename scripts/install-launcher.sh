#!/usr/bin/env bash
set -Eeuo pipefail

project_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
launch_path="$project_dir/scripts/launch.sh"
icon_path="$project_dir/icon.svg"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
if [[ "$data_home" != /* ]]; then
  data_home="$HOME/.local/share"
fi

if [[ ! -x "$launch_path" || ! -r "$icon_path" ]]; then
  printf 'The executable launch script or icon.svg is missing from the project.\n' >&2
  exit 1
fi
case "$project_dir" in
  *$'\n'*|*$'\r'*) printf 'The project path cannot contain line breaks.\n' >&2; exit 1 ;;
esac

# Two levels of escaping: the quoted Exec argument, then the Desktop Entry value.
# https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
escaped_launch="${launch_path//\\/\\\\}"
escaped_launch="${escaped_launch//\"/\\\"}"
escaped_launch="${escaped_launch//\`/\\\`}"
escaped_launch="${escaped_launch//\$/\\\$}"
escaped_launch="${escaped_launch//\\/\\\\}"
escaped_launch="${escaped_launch//%/%%}"

applications_dir="$data_home/applications"
icons_dir="$data_home/icons/hicolor/scalable/apps"
mkdir -p -- "$applications_dir" "$icons_dir"
install -m 644 -- "$icon_path" "$icons_dir/it.nexa.store.svg"
desktop_file="$applications_dir/it.nexa.store.desktop"
temporary_file="$(mktemp)"
trap 'rm -f -- "$temporary_file"' EXIT
cat > "$temporary_file" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Nexa Store
GenericName=App store
GenericName[it]=Negozio di app
Comment=Browse, install and manage apps from APT, Flatpak, Snap and AppImage
Comment[it]=Cerca, installa e gestisci app da APT, Flatpak, Snap e AppImage
Exec=/bin/bash "$escaped_launch"
Icon=it.nexa.store
Terminal=false
Categories=System;PackageManager;
StartupNotify=true
StartupWMClass=nexa-store
EOF
install -m 644 -- "$temporary_file" "$desktop_file"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications_dir" || true
fi
printf 'Nexa Store entry added to the user application menu:\n%s\n' "$desktop_file"
printf 'Keep the project in this folder, or run this script again after moving it.\n'
