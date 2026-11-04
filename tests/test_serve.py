import functools
import http.server
import threading
import urllib.request

import pytest

from perfectclone.serve import Handler
from perfectclone.util import site_dir


@pytest.fixture
def server(tmp_path):
    (tmp_path / "about").mkdir()
    (tmp_path / "index.html").write_text("home")
    (tmp_path / "about" / "index.html").write_text("about page")
    (tmp_path / "about" / "index.rsc").write_text('0:"about payload"\n')
    (tmp_path / "hero.png").write_bytes(b"png")
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(tmp_path)))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{httpd.server_address[1]}"
    httpd.shutdown()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def get(url, **headers):
    opener = urllib.request.build_opener(NoRedirect)
    try:
        r = opener.open(urllib.request.Request(url, headers=headers))
    except urllib.error.HTTPError as e:
        r = e
    return r.status, r.headers, r.read()


def test_clean_urls_without_trailing_slash_redirect(server):
    assert get(server + "/about")[::2] == (200, b"about page")


def test_next_navigation_payload_is_served_as_rsc(server):
    status, headers, body = get(server + "/about?_rsc=abc123")
    assert status == 200 and headers["Content-Type"] == "text/x-component" and body == b'0:"about payload"\n'


def test_next_image_maps_to_local_original_or_redirects_to_remote(server):
    assert get(server + "/_next/image?url=%2Fhero.png&w=640&q=75")[::2] == (200, b"png")
    status, headers, _ = get(server + "/_next/image?url=https%3A%2F%2Fcdn.cms.io%2Fa.jpg&w=640")
    assert status == 302 and headers["Location"] == "https://cdn.cms.io/a.jpg"


def test_byte_ranges_for_video(server):
    status, headers, body = get(server + "/hero.png", Range="bytes=1-2")
    assert status == 206 and headers["Content-Range"] == "bytes 1-2/3" and body == b"ng"
    status, headers, body = get(server + "/hero.png", Range="bytes=-2")
    assert status == 206 and body == b"ng"
    assert get(server + "/hero.png", Range="bytes=9-")[0] == 416
    assert get(server + "/hero.png")[1]["Accept-Ranges"] == "bytes"


def test_site_dir_accepts_the_clone_folder(tmp_path):
    (tmp_path / "site").mkdir()
    (tmp_path / "site" / "index.html").write_text("x")
    assert site_dir(tmp_path) == tmp_path / "site"
    assert site_dir(tmp_path / "site") == tmp_path / "site"
