#!/usr/bin/env bash
# Exercise selection and failure handling without Apple credentials or network.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin" "$work/Fixture.app"
: > "$work/env"
export ENV_FILE="$work/env"
export PATH="$work/bin:$PATH"
unset PATINAE_APPLE_SIGNING_IDENTITY PATINAE_APPLE_TEAMID
unset PATINAE_APPLE_EMAIL PATINAE_APPLE_APP_PASS PATINAE_APPLE_NOTARY_PROFILE
cat > "$work/bin/security" <<'SH'
#!/usr/bin/env bash
cat "$IDENTITIES"
SH
chmod +x "$work/bin/security"
export IDENTITIES="$work/identities"
cat > "$IDENTITIES" <<'IDENTITIES'
  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Apple Distribution: Example (ABCDEFGHIJ)"
  2) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB "Developer ID Application: Example (ABCDEFGHIJ)"
     2 valid identities found
IDENTITIES
source "$root/macos/signing-config.sh"
PATINAE_APPLE_TEAMID=ABCDEFGHIJ
select_signing_identity
[[ "$SIGNING_IDENTITY" == BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB ]]
PATINAE_APPLE_SIGNING_IDENTITY='Developer ID Application: Example (ABCDEFGHIJ)'
select_signing_identity
PATINAE_APPLE_SIGNING_IDENTITY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
if select_signing_identity 2>/dev/null; then echo 'Selected a non-Developer-ID certificate' >&2; exit 1; fi
unset PATINAE_APPLE_SIGNING_IDENTITY
PATINAE_APPLE_TEAMID=ZZZZZZZZZZ
if select_signing_identity 2>/dev/null; then echo 'Selected the wrong team' >&2; exit 1; fi
unset PATINAE_APPLE_TEAMID
cat >> "$IDENTITIES" <<'IDENTITIES'
  3) CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC "Developer ID Application: Other (ZZZZZZZZZZ)"
IDENTITIES
if select_signing_identity 2>/dev/null; then echo 'Accepted an ambiguous identity' >&2; exit 1; fi
echo 'PASS: Developer ID selection, full name, wrong type, wrong team and ambiguity'

# Skip compilation prerequisites: exercise the actual Make wrappers on a
# missing app, for which the strict signing operation must fail.
for target in sign sign-full; do
    make -s -C "$root" -o app -o app-full "$target" \
        "APP_DIR=$work/missing.app" "ENV_FILE=$work/env" > "$work/$target.log" 2>&1
    grep -q 'continuing with packaging' "$work/$target.log"
done
if make -s -C "$root" sign-existing "APP_DIR=$work/missing.app" \
    "ENV_FILE=$work/env" > "$work/strict.log" 2>&1; then
    echo 'Standalone signing hid a failure' >&2; exit 1
fi
echo 'PASS: Both build wrappers tolerate signing failure; standalone signing fails'

for tool in codesign ditto spctl; do
    printf '#!/usr/bin/env bash\nexit 0\n' > "$work/bin/$tool"
    chmod +x "$work/bin/$tool"
done
cat > "$work/bin/xcrun" <<'SH'
#!/usr/bin/env bash
case "$1 $2" in
    'notarytool submit') printf '{"id":"fixture-id","status":"%s"}\n' "$NOTARY_STATUS" ;;
    'notarytool log') printf '{}\n' > "${@: -1}" ;;
    'stapler staple'|'stapler validate') echo "$2" >> "$STAPLE_CALLS" ;;
    *) exit 1 ;;
esac
SH
chmod +x "$work/bin/xcrun"
export PATINAE_APPLE_NOTARY_PROFILE=fixture
export STAPLE_CALLS="$work/staples"
export NOTARY_STATUS=Invalid
if bash "$root/macos/notarize.sh" "$work/Fixture.app" > "$work/invalid.log" 2>&1; then
    echo 'Invalid submission was reported as success' >&2; exit 1
fi
[[ ! -e "$STAPLE_CALLS" ]]
[[ -n "$(find "$work" -name log.json -print)" ]]
make -s -C "$root" notarize "APP_DIR=$work/Fixture.app" "ENV_FILE=$work/env" > "$work/make.log" 2>&1
grep -q 'continuing without it' "$work/make.log"
echo 'PASS: Invalid status retains diagnostics, never staples, and does not fail make notarize'
export NOTARY_STATUS=Accepted
bash "$root/macos/notarize.sh" "$work/Fixture.app" > "$work/accepted.log" 2>&1
[[ "$(cat "$STAPLE_CALLS")" == $'staple\nvalidate' ]]
echo 'PASS: Accepted status staples and validates the artifact'
