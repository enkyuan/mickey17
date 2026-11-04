from nextpage import decoded, page_with

from perfectclone.edit import case_variants, localize, rehost, replace, replacer


def test_replace_skips_urls_and_paths():
    swap = replacer(case_variants("Acme", "Globex"))
    text = 'Acme ships. <img src="/acme/logo.png"> <a href="https://acme.com/Acme">ACME</a> acme'
    assert swap(text) == 'Globex ships. <img src="/acme/logo.png"> <a href="https://acme.com/Acme">GLOBEX</a> globex'


def test_whole_word_by_default():
    assert replacer([("Acme", "Globex")])("Acme Acmes heyAcme") == "Globex Acmes heyAcme"


def test_replace_edits_html_payload_and_rsc_together(tmp_path):
    page = tmp_path / "index.html"
    page.write_text(page_with('0:["$","h1",null,{"children":"Acme"}]\n'))
    rsc = tmp_path / "index.rsc"
    rsc.write_text("1:T4,Acme")
    replace(tmp_path, "Acme", "Globex", log=lambda *_: None)
    assert "<h1>Globex</h1>" in page.read_text()
    assert decoded(page.read_text()) == '0:["$","h1",null,{"children":"Globex"}]\n'
    assert rsc.read_text() == "1:T6,Globex"


def test_rehost_covers_www_and_http_variants(tmp_path):
    page = tmp_path / "index.html"
    page.write_text('<link rel="canonical" href="https://www.acme.com/x"/><meta content="http://acme.com"/>'
                    '<a href="https://acme.company.com">other site</a>')
    rehost(tmp_path, "https://acme.com", "https://globex.com", log=lambda *_: None)
    assert page.read_text() == ('<link rel="canonical" href="https://globex.com/x"/><meta content="https://globex.com"/>'
                                '<a href="https://acme.company.com">other site</a>')


def test_localize_rewrites_assets_but_not_links_or_metadata(tmp_path):
    page = tmp_path / "index.html"
    page.write_text(
        '<link rel="canonical" href="https://www.acme.com/"/>'
        '<link rel="stylesheet" href="https://acme.com/news/style.css"/>'
        '<style>@font-face{src:url(https://acme.com/fonts/a.woff2)}</style>'
        '<script type="importmap">{"imports":{"a":"https://acme.com/a.js"}}</script>'
        '<img src="https://www.acme.com/logo.png"><a href="https://acme.com/about">About</a>'
        '<img src="https://cdn.other.com/x.png">'
    )
    (tmp_path / "style.css").write_text("a{background:url('https://acme.com/bg.png')}")
    localize(tmp_path, "https://acme.com")
    assert page.read_text() == (
        '<link rel="canonical" href="https://www.acme.com/"/>'
        '<link rel="stylesheet" href="/news/style.css"/>'
        '<style>@font-face{src:url(/fonts/a.woff2)}</style>'
        '<script type="importmap">{"imports":{"a":"/a.js"}}</script>'
        '<img src="/logo.png"><a href="https://acme.com/about">About</a>'
        '<img src="https://cdn.other.com/x.png">'
    )
    assert (tmp_path / "style.css").read_text() == "a{background:url('/bg.png')}"
