#!/usr/bin/env bash
# Bake the store images from promo.html with the Chrome that is on this machine.
#
# The store accepts a screenshot of 1280x800 (or 640x400), a small tile of 440x280 and a
# marquee of 1400x560. Every image comes from one HTML file, so a change of the words is a
# change of text and not of an image editor.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${here}/../assets"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
mkdir -p "${out}"

bake() { # frame-id width height name
  "${chrome}" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size="${2},${3}" --screenshot="${out}/${4}.png" \
    "file://${here}/promo.html?frame=${1}" >/dev/null 2>&1
  echo "${out}/${4}.png"
}

bake shot1 1280 800 screenshot-1-player
bake shot2 1280 800 screenshot-2-list
bake shot3 1280 800 screenshot-3-work
bake shot4 1280 800 screenshot-4-top
bake tile 440 280 promo-tile-small
bake marquee 1400 560 promo-marquee
