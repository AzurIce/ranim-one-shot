#!/usr/bin/env bash
# Evenly sample frames from a rendered video for visual inspection.
#
# Usage: sample-frames.sh <video.mp4> <outdir> [count=24]
#
# Writes <outdir>/t<seconds>.jpg files, first frame at ~1s and last at
# duration-0.5s, plus the exact times printed to stdout. Read every frame
# back with the visual tool — do not assume the render is correct.
set -euo pipefail

video=${1:?usage: sample-frames.sh <video.mp4> <outdir> [count]}
out=${2:?usage: sample-frames.sh <video.mp4> <outdir> [count]}
n=${3:-24}

command -v ffmpeg >/dev/null || { echo "ffmpeg not in PATH (nix develop provides it)" >&2; exit 1; }
mkdir -p "$out"

dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$video")
step=$(python3 -c "d=$dur; n=$n; print(max((d - 1.5) / (n - 1), 0.5))")

times=()
for i in $(seq 0 $((n - 1))); do
  t=$(python3 -c "d=$dur; s=$step; print(round(min(1.0 + $i * s, d - 0.5), 2))")
  times+=("$t")
  ffmpeg -y -v error -ss "$t" -i "$video" -frames:v 1 "$out/$(printf 't%08.2f' "$t").jpg"
done

printf 'sampled %s frames (%ss video):\n' "$n" "$dur"
printf '  %s\n' "${times[@]}"
