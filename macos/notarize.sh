#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/signing-config.sh"

artifact=${1:?Usage: notarize.sh path/to/Patinae.app-or-dmg}
[[ -e "$artifact" ]] || { echo "Missing artifact: $artifact" >&2; exit 1; }
command -v jq >/dev/null || { echo "Notarization requires jq" >&2; exit 1; }
auth=()
if [[ -n "${PATINAE_APPLE_NOTARY_PROFILE:-}" ]]; then
    auth=(--keychain-profile "$PATINAE_APPLE_NOTARY_PROFILE")
elif [[ -n "${PATINAE_APPLE_EMAIL:-}" && -n "${PATINAE_APPLE_APP_PASS:-}" && -n "${PATINAE_APPLE_TEAMID:-}" ]]; then
    [[ "$PATINAE_APPLE_TEAMID" =~ ^[A-Z0-9]{10}$ ]] || {
        echo "PATINAE_APPLE_TEAMID must be the 10-character Team ID, not a certificate name." >&2; exit 1;
    }
    auth=(--apple-id "$PATINAE_APPLE_EMAIL" --team-id "$PATINAE_APPLE_TEAMID" --password "$PATINAE_APPLE_APP_PASS")
else
    echo "Set PATINAE_APPLE_NOTARY_PROFILE or Apple email, Team ID and app-specific password." >&2
    exit 1
fi

codesign --verify --deep --strict "$artifact"
report=$(mktemp -d "$(dirname "$artifact")/notarization.XXXXXX")
upload=$artifact
if [[ "$artifact" == *.app ]]; then
    upload="$report/submission.zip"
    ditto -c -k --keepParent "$artifact" "$upload"
fi
# Retain JSON diagnostics and the submission ID, but not the temporary archive.
trap 'if [[ "$upload" != "$artifact" ]]; then rm -f "$upload"; fi' EXIT
echo "Notarization reports: $report"
result=0
xcrun notarytool submit "$upload" "${auth[@]}" --wait \
    --timeout "${PATINAE_NOTARY_TIMEOUT:-20m}" --output-format json \
    > "$report/submission.json" || result=$?
submission_id=$(jq -r '.id // empty' "$report/submission.json")
status=$(jq -r '.status // empty' "$report/submission.json")
if [[ -n "$submission_id" ]]; then
    echo "Submission: $submission_id; status: ${status:-unknown}"
    xcrun notarytool log "$submission_id" "${auth[@]}" "$report/log.json" || true
fi
if [[ "$result" != 0 || "$status" != Accepted ]]; then
    echo "Notarization did not succeed. Inspect $report (a timed-out submission may still be processing)." >&2
    exit 1
fi
xcrun stapler staple "$artifact"
xcrun stapler validate "$artifact"
if [[ "$artifact" == *.app ]]; then
    spctl --assess --type execute --verbose=2 "$artifact"
fi
echo "Notarized and stapled: $artifact"
