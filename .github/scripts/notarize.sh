#!/usr/bin/env bash
set -euo pipefail
result="$(xcrun notarytool submit "$1" --key "$RUNNER_TEMP/AuthKey.p8" --key-id "$API_KEY" --issuer "$API_ISSUER" --wait --timeout 30m --output-format json)"
echo "$result"
status="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')"
test "$status" = "Accepted"
