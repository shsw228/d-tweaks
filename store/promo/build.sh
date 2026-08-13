#!/usr/bin/env bash
# Bake every store image with the Chrome of this machine.
#
# 1. The settings page is rendered from the real WASM UI (`settings-preview.html` with
#    `mock-chrome.js`), because `chrome-extension://` cannot be opened outside the browser.
#    It needs a server: the page loads an ES module and a .wasm.
# 2. `make.py` inlines the captures into `promo.html`.
# 3. Chrome bakes one image per frame.
#
# The store accepts a screenshot of 1280x800 (or 640x400), a small tile of 440x280 and a
# marquee of 1400x560.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="${here}/../.."
out="${here}/../assets"
shots="${here}/../screenshots"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
port="${PORT:-8731}"
mkdir -p "${out}"

shoot() { # url width height file
  "${chrome}" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --virtual-time-budget=6000 --window-size="${2},${3}" --screenshot="${4}" "${1}" \
    >/dev/null 2>&1
}

# --- 1. the settings page, from the real UI ---
python3 -m http.server "${port}" -d "${root}" >/dev/null 2>&1 &
server=$!
trap 'kill "${server}" 2>/dev/null || true' EXIT
sleep 1
# The group with the most rows, so the picture is not a group with one row
shoot "http://localhost:${port}/store/promo/settings-preview.html?group=%E4%B8%80%E8%A6%A7" \
  1280 900 "${shots}/raw-05-settings.png"
kill "${server}" 2>/dev/null || true
trap - EXIT

# --- 2. and 3. the store images ---
python3 "${here}/make.py"
# One set per language: the store keeps a listing per language, and the images belong to it
for lang in ja en; do
  mkdir -p "${out}/${lang}"
  for frame in shot1:screenshot-1-player shot2:screenshot-2-list shot3:screenshot-3-work \
               shot4:screenshot-4-top shot5:screenshot-5-settings; do
    shoot "file://${here}/promo.html?frame=${frame%%:*}-${lang}" 1280 800 \
      "${out}/${lang}/${frame##*:}.png"
    echo "${out}/${lang}/${frame##*:}.png"
  done
  shoot "file://${here}/promo.html?frame=tile-${lang}" 440 280 \
    "${out}/${lang}/promo-tile-small.png"
  echo "${out}/${lang}/promo-tile-small.png"
  shoot "file://${here}/promo.html?frame=marquee-${lang}" 1400 560 \
    "${out}/${lang}/promo-marquee.png"
  echo "${out}/${lang}/promo-marquee.png"
done
