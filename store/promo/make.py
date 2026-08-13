#!/usr/bin/env python3
"""Write promo.html with the captures inlined.

The captures live in `store/screenshots`. They are inlined as base64 so that headless
Chrome can bake the images from `file://` without a server and without a missing image.
"""
import base64
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
SHOTS = HERE.parent / "screenshots"

# (frame id, capture, {language: (heading, line under it)})
#
# The store keeps one listing per language, and a screenshot belongs to a listing, so every
# frame exists in both languages. The captures are the same: the site is Japanese.
FRAMES = [
    ("shot1", "raw-02-player.jpg", {
        "ja": ("話の途中でも、一覧の上で再生",
               "ニコニコの同じ話のコメントを重ねる。章立てとコメント量はシークバーの下に。"),
        "en": ("Play an episode over the list",
               "Comments of the same episode from nicovideo. Chapters and comment density under the seek bar."),
    }),
    ("shot2", "raw-01-list.jpg", {
        "ja": ("2 列 860px を、画面いっぱいのグリッドに",
               "ページ送りは無限スクロール。視聴済みと進捗はカードの上。"),
        "en": ("Two columns in 860px become a full-width grid",
               "The paging becomes an infinite scroll. What you watched, and how far, is on the card."),
    }),
    ("shot3", "raw-03-work.jpg", {
        "ja": ("エピソードは折りたたまず、全話ならべる",
               "見出しは全幅のヒーロー。あらすじ・キャスト・スタッフは表に。"),
        "en": ("Every episode at once, nothing folded",
               "The head becomes a full-width hero. Summary, cast and staff become tables."),
    }),
    ("shot4", "raw-04-top.jpg", {
        "ja": ("15 本の横スクロールを、1 画面に",
               "ランキングのショーケース、今日の更新、チップで切り替えるグリッド。"),
        "en": ("15 horizontal strips become one screen",
               "A showcase of the ranking, the episodes of today, and one grid with chips."),
    }),
    ("shot5", "raw-05-settings.png", {
        "ja": ("11 の機能は、すべて個別に切れる",
               "設定は場所ごとに分けてある。全体スイッチ 1 つでサイト本来の表示に戻る。"),
        "en": ("Eleven features, each with its own switch",
               "The settings are grouped by place. One main switch gives the original site back."),
    }),
]

# The name and the line of the tile and the marquee
BRAND = {
    "ja": ("dアニメストアの PC 表示を作り変える",
           "全幅グリッド・全話ならべ・同じ画面で再生・ニコニコのコメント"),
    "en": ("Rebuild the PC web of dアニメストア",
           "Full-width grids, every episode, playback on the same page, nicovideo comments"),
}


def data_uri(name: str) -> str:
    path = SHOTS / name
    kind = "png" if path.suffix == ".png" else "jpeg"
    return f"data:image/{kind};base64," + base64.b64encode(path.read_bytes()).decode()


def main() -> None:
    cards = "\n".join(
        f'<section class="frame" data-frame="{fid}-{lang}">\n'
        f"  <div class=\"copy\"><h2>{words[lang][0]}</h2><p>{words[lang][1]}</p></div>\n"
        f'  <div class="shot"><img src="{data_uri(shot)}" alt=""></div>\n'
        f"</section>"
        for fid, shot, words in FRAMES
        if (SHOTS / shot).exists()
        for lang in words
    )
    # The tile and the marquee carry only words, so they are built here
    for lang, (sub_tile, sub_marquee) in BRAND.items():
        cards += (
            f'\n<section class="frame" data-frame="tile-{lang}">'
            f'<div class="name">d-tweaks</div><div class="rule"></div>'
            f'<div class="sub">{sub_tile}</div></section>'
            f'\n<section class="frame" data-frame="marquee-{lang}">'
            f'<div class="name">d-tweaks</div><div class="rule"></div>'
            f'<div class="sub">{sub_marquee}</div></section>'
        )
    template = (HERE / "template.html").read_text()
    (HERE / "promo.html").write_text(template.replace("<!--FRAMES-->", cards))
    print("promo.html", len((HERE / "promo.html").read_text()) // 1024, "KB")


if __name__ == "__main__":
    main()
