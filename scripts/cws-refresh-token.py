#!/usr/bin/env python3
"""Get a refresh token for the Chrome Web Store API.

The old flow that showed the code on a page (`urn:ietf:wg:oauth:2.0:oob`) is gone, so the
code has to arrive on a redirect. This starts a listener on localhost, opens the browser,
takes the code and exchanges it for a refresh token.

    scripts/cws-refresh-token.py ~/Downloads/client_secret_….json --set-secrets

The argument is the file that the Cloud console gives for an OAuth client of the type
"Desktop app" (it has an `installed` object). With `--set-secrets` the three values go
straight into the repository secrets with `gh secret set`, so no secret is printed and none
stays in the history of the shell. Without it, the three lines are printed.

Put the publishing status of the consent screen to "In production": in "Testing" a refresh
token dies after seven days.
"""

import http.server
import pathlib
import json
import subprocess
import sys
import urllib.parse
import urllib.request
import webbrowser

PORT = 8721
REDIRECT = f"http://localhost:{PORT}"
SCOPE = "https://www.googleapis.com/auth/chromewebstore"
AUTH = "https://accounts.google.com/o/oauth2/auth"
TOKEN = "https://oauth2.googleapis.com/token"

args = [a for a in sys.argv[1:] if not a.startswith("--")]
set_secrets = "--set-secrets" in sys.argv
if len(args) != 1:
    sys.exit(__doc__)

data = json.loads(pathlib.Path(args[0]).expanduser().read_text())
client = data.get("installed")
if not client:
    sys.exit(
        "その JSON は『デスクトップ アプリ』のものではありません"
        f"（持っているのは {list(data)}）。種類を選び直してください。"
    )
client_id = client["client_id"]
client_secret = client["client_secret"]

code_holder: dict[str, str] = {}


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 (the name comes from the library)
        query = urllib.parse.urlparse(self.path).query
        params = urllib.parse.parse_qs(query)
        code_holder.update({k: v[0] for k, v in params.items()})
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.end_headers()
        done = "code" in code_holder
        self.wfile.write(
            ("受け取りました。ターミナルに戻ってください。" if done else "コードがありません。").encode()
        )

    def log_message(self, *_args) -> None:
        pass  # the console is for the result, not for the requests


url = f"{AUTH}?" + urllib.parse.urlencode(
    {
        "client_id": client_id,
        "redirect_uri": REDIRECT,
        "response_type": "code",
        "scope": SCOPE,
        "access_type": "offline",
        # Without this, a second run gives no refresh token
        "prompt": "consent",
    }
)
print("ブラウザで許可してください。開かないときはこの URL を開いてください:")
print(url)
webbrowser.open(url)

with http.server.HTTPServer(("localhost", PORT), Handler) as server:
    server.handle_request()

if "code" not in code_holder:
    sys.exit(f"コードが来ませんでした: {code_holder}")

body = urllib.parse.urlencode(
    {
        "code": code_holder["code"],
        "client_id": client_id,
        "client_secret": client_secret,
        "redirect_uri": REDIRECT,
        "grant_type": "authorization_code",
    }
).encode()
with urllib.request.urlopen(urllib.request.Request(TOKEN, data=body)) as response:
    token = json.load(response)

refresh = token.get("refresh_token")
if not refresh:
    sys.exit(f"refresh_token がありません（prompt=consent を確認）: {token}")

values = {
    "CWS_CLIENT_ID": client_id,
    "CWS_CLIENT_SECRET": client_secret,
    "CWS_REFRESH_TOKEN": refresh,
}
print()
if not set_secrets:
    print("--- これを Secrets に入れてください ---")
    for name, value in values.items():
        print(f"{name}={value}")
    sys.exit(0)

# The value goes over stdin, so it is not in the arguments of the process
for name, value in values.items():
    result = subprocess.run(
        ["gh", "secret", "set", name],
        input=value,
        text=True,
        capture_output=True,
        check=False,
    )
    state = "ok" if result.returncode == 0 else f"失敗: {result.stderr.strip()}"
    print(f"{name}: {state}")
print()
print("アイテム ID も入れてください: gh secret set CWS_EXTENSION_ID")
