#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/signing-config.sh"

app=${1:?Usage: sign-app.sh path/to/Patinae.app}
[[ -f "$app/Contents/Info.plist" && -f "$app/Contents/MacOS/patinae" ]] || {
    echo "Not a complete Patinae app: $app" >&2; exit 1;
}
select_signing_identity

# Inspect file contents, not extensions: Python distributions can contain
# additional executables. Sign leaves before enclosing bundles and the app.
while IFS= read -r -d '' binary; do
    kind=$(file -b "$binary")
    case "$kind" in
        *Mach-O*executable*|*Mach-O*shared\ library*|*Mach-O*bundle*)
            codesign --force --timestamp --options runtime \
                --sign "$SIGNING_IDENTITY" "$binary"
            codesign --verify --strict "$binary"
            ;;
    esac
done < <(find "$app/Contents" -type f -print0)

while IFS= read -r -d '' bundle; do
    codesign --force --timestamp --options runtime \
        --sign "$SIGNING_IDENTITY" "$bundle"
done < <(find "$app/Contents" -depth -type d \
    \( -name '*.app' -o -name '*.framework' -o -name '*.bundle' -o -name '*.xpc' -o -name '*.appex' \) -print0)

# Patinae intentionally loads user-installed native plugins.
codesign --force --timestamp --options runtime \
    --entitlements "$SIGNING_DIR/Patinae.entitlements" \
    --sign "$SIGNING_IDENTITY" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
codesign --display --verbose=2 "$app"
echo "Signed and verified: $app"
