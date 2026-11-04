from nextpage import decoded, page_with

from perfectclone.flight import edit_json_rows
from perfectclone.trackers import _is_tracker_element, _strip_html, strip


def test_tracker_elements_are_dropped_from_payload():
    pixel = '["$","$L5",null,{"id":"meta-pixel","strategy":"afterInteractive","children":"fbq(\'init\')"}]'
    app = '["$","$L5",null,{"src":"https://example.com/app.js","strategy":"afterInteractive"}]'
    stream = f'0:["$","body",null,{{"children":[{pixel},{app}]}}]\n'
    new = decoded(edit_json_rows(page_with(stream), lambda n: None if _is_tracker_element(n) else n))
    assert "meta-pixel" not in new and "example.com/app.js" in new


def test_strip_html_keeps_payload_and_jsonld():
    html = ('<link rel="preload" href="https://snap.licdn.com/x.js" as="script"/>'
            '<script async src="https://www.googletagmanager.com/gtag/js?id=G-1"></script>'
            '<script type="application/ld+json">{"a":1}</script><script src="/app.js"></script>')
    assert _strip_html(html) == '<script type="application/ld+json">{"a":1}</script><script src="/app.js"></script>'


def test_tracker_noscript_is_emptied_not_removed():
    html = '<body><noscript><iframe src="https://www.googletagmanager.com/ns.html?id=GTM-X"></iframe></noscript><main></main></body>'
    assert _strip_html(html) == "<body><noscript></noscript><main></main></body>"


def test_runtime_loader_urls_in_bundles_are_neutralized(tmp_path):
    js = tmp_path / "layout.js"
    js.write_text("j.src='https://www.googletagmanager.com/gtm.js?id='+i;fetch('https://api.example.com/x')")
    strip(tmp_path, log=lambda *_: None)
    assert js.read_text() == "j.src='data:text/javascript,//www.googletagmanager.com/gtm.js?id='+i;fetch('https://api.example.com/x')"
