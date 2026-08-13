#!/usr/bin/env python3
"""Write promo.html with the captures inlined.

The captures live in `store/screenshots`. They are inlined as base64 so that headless
Chrome can bake the images from `file://` without a server and without a missing image.
"""
import base64
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
SHOTS = HERE.parent / "screenshots"

# (frame id, capture, heading, line under it)
FRAMES = [
    ("shot1", "raw-02-player.jpg", "話の途中でも、一覧の上で再生",
     "ニコニコの同じ話のコメントを重ねる。章立てとコメント量はシークバーの下に。"),
    ("shot2", "raw-01-list.jpg", "2 列 860px を、画面いっぱいのグリッドに",
     "ページ送りは無限スクロール。視聴済みと進捗はカードの上。"),
    ("shot3", "raw-03-work.jpg", "エピソードは折りたたまず、全話ならべる",
     "見出しは全幅のヒーロー。あらすじ・キャスト・スタッフは表に。"),
    ("shot4", "raw-04-top.jpg", "15 本の横スクロールを、1 画面に",
     "ランキングのショーケース、今日の更新、チップで切り替えるグリッド。"),
    ("shot5", "raw-05-settings.png", "11 の機能は、すべて個別に切れる",
     "設定は場所ごとに分けてある。全体スイッチ 1 つでサイト本来の表示に戻る。"),
]


def data_uri(name: str) -> str:
    path = SHOTS / name
    kind = "png" if path.suffix == ".png" else "jpeg"
    return f"data:image/{kind};base64," + base64.b64encode(path.read_bytes()).decode()


def main() -> None:
    cards = "\n".join(
        f'<section class="frame" data-frame="{fid}">\n'
        f'  <div class="copy"><h2>{head}</h2><p>{line}</p></div>\n'
        f'  <div class="shot"><img src="{data_uri(shot)}" alt=""></div>\n'
        f"</section>"
        for fid, shot, head, line in FRAMES
        if (SHOTS / shot).exists()
    )
    template = (HERE / "template.html").read_text()
    (HERE / "promo.html").write_text(template.replace("<!--FRAMES-->", cards))
    print("promo.html", len((HERE / "promo.html").read_text()) // 1024, "KB")


if __name__ == "__main__":
    main()
