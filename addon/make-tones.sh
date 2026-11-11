#!/usr/bin/env bash
set -euo pipefail

FREQS=(1300 1700 2150 2550 2950 3350 3750 4150)
DURATION=0.15
AMPLITUDE=0.25
RATE=44100
OUT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/WarcryBeacon/tones"

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
mkdir -p "$OUT"

for index in "${!FREQS[@]}"; do
  freq="${FREQS[$index]}"
  fade_out=$(awk -v d="$DURATION" 'BEGIN { printf "%.3f", d - 0.02 }')
  ffmpeg -loglevel error -y \
    -f lavfi -i "sine=frequency=${freq}:duration=${DURATION}:sample_rate=${RATE}" \
    -af "volume=${AMPLITUDE},afade=t=in:st=0:d=0.01,afade=t=out:st=${fade_out}:d=0.02" \
    -ac 2 -strict -2 -c:a vorbis -q:a 6 "$OUT/t${index}.ogg"
  echo "t${index}.ogg  ${freq} Hz"
done
