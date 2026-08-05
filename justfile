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

# バージョンを上げて、コミットとタグまで作る。**push はしない。**
#
# ■ バージョンの持ち主は manifest.json（と Cargo.toml）で、タグはその印
#
#   タグを起点に CI が動く。タグが指すコミットの中に正しい version が入っている
#   必要があるので、順序は「上げる → コミット → タグ」で固定する。
#   CI 側（.github/workflows/release.yml）はタグ名と manifest の version が
#   一致しなければ落ちる。
#
# ■ Chrome のバージョン表記
#
#   1〜4 個の整数をドットで繋いだ形だけ。各値は 0〜65535、先頭ゼロ不可。
#   `-rc.1` のような接尾辞は使えない（semver の一部が使えない点に注意）。
#   ストアは同じ版の再アップロードを受け付けないので、必ず上げる。
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    version="{{version}}"

    python3 - "${version}" <<'PY'
    import re, sys
    v = sys.argv[1]
    parts = v.split(".")
    if not 1 <= len(parts) <= 4:
        sys.exit(f"バージョンは 1〜4 個の整数です: {v}")
    for part in parts:
        if not re.fullmatch(r"0|[1-9][0-9]{0,4}", part) or int(part) > 65535:
            sys.exit(f"各値は 0〜65535 の整数で、先頭ゼロは使えません: {v}")
    PY

    if [ -n "$(git status --porcelain)" ]; then
      echo "作業ツリーに未コミットの変更があります" >&2
      exit 1
    fi
    if git rev-parse -q --verify "refs/tags/${version}" >/dev/null; then
      echo "タグ ${version} は既にあります" >&2
      exit 1
    fi

    python3 - "${version}" <<'PY'
    import json, pathlib, re, sys
    version = sys.argv[1]

    manifest = pathlib.Path("extension/manifest.json")
    text = manifest.read_text()
    text = re.sub(r'("version":\s*")[^"]+(")', rf'\g<1>{version}\g<2>', text, count=1)
    manifest.write_text(text)
    assert json.loads(text)["version"] == version

    cargo = pathlib.Path("Cargo.toml")
    text = cargo.read_text()
    text = re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', text, count=1)
    cargo.write_text(text)
    PY

    just check
    just test
    just package

    git add -A
    git commit -m "[chore] バージョンを ${version} に上げる"
    git tag "${version}"
    echo "--- タグ ${version} を作りました ---"
    echo "公開するときは: git push origin main ${version}"

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
