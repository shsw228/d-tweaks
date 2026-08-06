set shell := ["bash", "-euo", "pipefail", "-c"]

# 既定はデバッグビルド
default: dev

# 開発ビルド
dev:
    @just _build-core --dev
    @just _build-background --dev
    @just _build-options --dev

# リリースビルド（opt-level=z + wasm-opt -Oz）
build:
    @just _build-core --release
    @just _build-background --release
    @just _build-options --release
    @echo "--- 成果物 ---"
    @ls -lh extension/pkg/core_bg.wasm extension/pkg-background/background_bg.wasm extension/pkg-options/options_bg.wasm

# content script。ES module は使えないので no-modules で出す。
_build-core mode:
    wasm-pack build crates/core \
      --target no-modules \
      --out-dir ../../extension/.stage-pkg \
      --out-name core \
      {{mode}}
    @just _swap extension/.stage-pkg extension/pkg

# service worker は "type": "module" なので web ターゲットが使える。
_build-background mode:
    wasm-pack build crates/background \
      --target web \
      --out-dir ../../extension/.stage-pkg-background \
      --out-name background \
      {{mode}}
    @just _swap extension/.stage-pkg-background extension/pkg-background

# options page も module script。
_build-options mode:
    wasm-pack build crates/options \
      --target web \
      --out-dir ../../extension/.stage-pkg-options \
      --out-name options \
      {{mode}}
    @just _swap extension/.stage-pkg-options extension/pkg-options

# 組み終わったものを本番の場所へ入れ替える。
#
# ■ なぜ直接書かせないのか
#
#   wasm-pack は .js と .wasm を順に書くので、**書いている途中にページを読み込むと
#   新しい .js と古い .wasm が混ざる**。この組み合わせは import 名のハッシュが
#   合わないので、こうなる:
#
#     LinkError: WebAssembly.instantiate(): Import #8 "./core_bg.js"
#     "__wbg_set_onload_84cd766c68d572d8": function import requires a callable
#
#   ディレクトリごと rename で入れ替えれば、外から見えるのは「古い一式」か
#   「新しい一式」のどちらかだけになる。混ざった状態を踏めなくなる。
#   npm 向けのファイル（package.json など）もここで落とす。
_swap staged live:
    @rm -f {{staged}}/package.json {{staged}}/.gitignore {{staged}}/README.md
    @rm -rf {{live}}.old
    @if [ -d {{live}} ]; then mv {{live}} {{live}}.old; fi
    @mv {{staged}} {{live}}
    @rm -rf {{live}}.old

# Chrome ウェブストアに出す zip を作る。
#
# ■ 入れてはいけないもの
#
#   - `_metadata/` … Chrome が unpacked 読み込み時に作る DNR の索引。
#     `_` 始まりは予約名なので、入っているとアップロードで弾かれる。
#   - `__MACOSX/` `.DS_Store` … macOS が zip に混ぜる。`-X` と除外で落とす。
#   - `*.d.ts` … wasm-pack が出す型定義。実行には要らない。
#
# 出力は dist/d-tweaks-<manifest の version>.zip。
package: build
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(python3 -c 'import json;print(json.load(open("extension/manifest.json"))["version"])')
    out="dist/d-tweaks-${version}.zip"
    rm -rf dist/stage "${out}"
    mkdir -p dist/stage
    cp -R extension/ dist/stage/
    rm -rf dist/stage/_metadata dist/stage/.stage-pkg* dist/stage/pkg*.old
    find dist/stage -name '*.d.ts' -delete
    find dist/stage -name '.DS_Store' -delete
    (cd dist/stage && zip -q -r -X "../../${out}" . -x '__MACOSX/*')
    rm -rf dist/stage
    # 入ってはいけないものが 1 つでもあれば、ここで落とす（CI もこれを見ている）
    if unzip -Z1 "${out}" | grep -E '(^_metadata/|^__MACOSX/|\.d\.ts$|\.DS_Store$)'; then
      echo "配布 zip に入れてはいけないものがあります" >&2
      exit 1
    fi
    echo "--- ${out} ---"
    unzip -l "${out}"

# ロジックのユニットテスト（ネイティブターゲット）
test:
    cargo test --workspace

check:
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt --check

fmt:
    cargo fmt

clean:
    cargo clean
    rm -rf extension/pkg extension/pkg-background extension/pkg-options
    rm -rf extension/.stage-pkg extension/.stage-pkg-background extension/.stage-pkg-options
    rm -rf extension/pkg.old extension/pkg-background.old extension/pkg-options.old
