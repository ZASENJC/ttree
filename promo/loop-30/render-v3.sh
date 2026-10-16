#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
python3 synth-v3.py
npx remotion render src/v3.tsx TTreeV3 out/picture-v3.mp4 --codec h264 --crf 17 --pixel-format yuv420p --concurrency=4 "$@"
ffmpeg -v error -y -i out/picture-v3.mp4 -i public/flow-v3.wav -map 0:v:0 -map 1:a:0 -c:v copy -c:a aac -b:a 320k -t 30 -movflags +faststart out/TTREE-30s-v3-vertical.mp4
