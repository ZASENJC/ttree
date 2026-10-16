#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
python3 synth.py
npx remotion render src/index.tsx TTree out/picture.mp4 --codec h264 --crf 17 --pixel-format yuv420p --concurrency=4 "$@"
ffmpeg -y -i out/picture.mp4 -i public/flow-original.wav -map 0:v:0 -map 1:a:0 -c:v copy -c:a aac -b:a 320k -t 30 -movflags +faststart out/TTREE-30s.mp4
