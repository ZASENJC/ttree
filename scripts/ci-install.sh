#!/usr/bin/env bash

set -euo pipefail

# Install the locked tree without running arbitrary dependency lifecycle hooks,
# then enable only the two native packages the application actually needs.
npm ci --ignore-scripts
npm rebuild esbuild fsevents
