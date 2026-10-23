#!/usr/bin/env bash
# Shared configuration for the signing tools. Never enable shell tracing here.
set +x
SIGNING_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ENV_FILE=${ENV_FILE:-"$SIGNING_DIR/../.env"}
if [[ -f "$ENV_FILE" ]]; then
    source "$ENV_FILE"
fi

select_signing_identity() {
    local identities matches count
    identities=$(security find-identity -v -p codesigning)
    matches=$(printf '%s\n' "$identities" | awk \
        -v requested="${PATINAE_APPLE_SIGNING_IDENTITY:-}" \
        -v team="${PATINAE_APPLE_TEAMID:-}" '
        /"Developer ID Application: / {
            name=$0; sub(/^[^"]*"/, "", name); sub(/".*$/, "", name)
            if (requested != "" && requested != $2 && requested != name) next
            if (team != "" && index(name, "(" team ")") == 0) next
            print $2
        }')
    count=$(printf '%s\n' "$matches" | awk 'NF {n++} END {print n+0}')
    if [[ "$count" != 1 ]]; then
        echo "Expected one matching Developer ID Application identity; found $count." >&2
        echo "Set PATINAE_APPLE_SIGNING_IDENTITY to its SHA-1 fingerprint; check keychain access and Team ID." >&2
        return 1
    fi
    SIGNING_IDENTITY=$matches
}
