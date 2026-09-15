#!/bin/bash
# Package an already assembled (and, for releases, signed/notarized) app.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "Usage: $0 /path/to/Patinae.app /path/to/Patinae.dmg" >&2
    exit 2
fi

app=$1
output=$2
script_dir=$(cd "$(dirname "$0")" && pwd)
create_dmg=${CREATE_DMG:-create-dmg}

if ! command -v "$create_dmg" >/dev/null 2>&1; then
    echo "Missing create-dmg. Install it with: brew install create-dmg" >&2
    exit 1
fi
if [[ ! -f "$app/Contents/Info.plist" || ! -x "$app/Contents/MacOS/patinae" ]]; then
    echo "Expected a complete Patinae.app: $app" >&2
    exit 1
fi

# Background coordinates are in Finder points; the TIFF contains 1x and 2x
# representations so text and artwork remain sharp on Retina displays.
window_width=768
background_height=512
window_height=544 # 512-point content area plus the Finder title bar.
icon_size=128
icon_y=257
app_x=205
applications_x=563

mkdir -p "$(dirname "$output")"
work=$(mktemp -d "$(dirname "$output")/.patinae-dmg.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir "$work/staging"
ditto "$app" "$work/staging/Patinae.app"
sips -z "$background_height" "$window_width" \
    "$script_dir/dmg-background@2x.png" --out "$work/background.png" >/dev/null
tiffutil -cathidpicheck "$work/background.png" \
    "$script_dir/dmg-background@2x.png" -out "$work/background.tiff" >/dev/null

"$create_dmg" \
    --volname "Patinae" \
    --volicon "$app/Contents/Resources/AppIcon.icns" \
    --background "$work/background.tiff" \
    --window-pos 200 120 \
    --window-size "$window_width" "$window_height" \
    --icon-size "$icon_size" \
    --text-size 14 \
    --icon "Patinae.app" "$app_x" "$icon_y" \
    --hide-extension "Patinae.app" \
    --app-drop-link "$applications_x" "$icon_y" \
    --format UDZO \
    "$work/Patinae.dmg" "$work/staging"

# Preserve any previous good image until packaging succeeds.
mv -f "$work/Patinae.dmg" "$output"
echo "Created $output"
