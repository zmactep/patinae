#!/usr/bin/env bash
# Publish the verified image from the manual re-notarization workflow.
set -euo pipefail
cd "${RUNNER_TEMP:?}"
release_api="repos/${GITHUB_REPOSITORY:?}/releases"
release_id=$(jq -er '.id' release.json)
tag=$(jq -er '.tag_name' release.json)
staged="Patinae-notarized-${GITHUB_RUN_ID:?}-${GITHUB_RUN_ATTEMPT:?}.dmg"
backup="Patinae-original-${ORIGINAL_ASSET:?}.dmg"

# Upload alongside the original so an interrupted transfer keeps the download working.
mv Patinae.dmg "$staged"
gh release upload "$tag" "$staged" --repo "$GITHUB_REPOSITORY"
gh api "$release_api/$release_id" > uploaded.json
digest="sha256:$(shasum -a 256 "$staged" | awk '{print $1}')"
new_id=$(jq -er --arg name "$staged" --arg digest "$digest" \
    '.assets[] | select(.name == $name and .digest == $digest and .state == "uploaded") | .id' uploaded.json)
jq -e --argjson id "$ORIGINAL_ASSET" \
    '.assets[] | select(.id == $id and .name == "Patinae.dmg")' uploaded.json >/dev/null

rename_asset() {
    gh api --method PATCH "$release_api/assets/$1" -f name="$2" --silent
}

rename_asset "$ORIGINAL_ASSET" "$backup"
if ! rename_asset "$new_id" Patinae.dmg; then
    echo "::error::Could not promote $staged. Restoring $backup."
    rename_asset "$ORIGINAL_ASSET" Patinae.dmg
    gh api --method DELETE "$release_api/assets/$new_id" --silent
    exit 1
fi
gh api --method DELETE "$release_api/assets/$ORIGINAL_ASSET" --silent
echo "Published Patinae.dmg ($digest)." >> "$GITHUB_STEP_SUMMARY"
