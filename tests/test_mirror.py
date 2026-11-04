from perfectclone.mirror import asset_path, page_path, refs

BASE = "https://acme.com/"


def test_encoded_dot_segments_cannot_escape_output(tmp_path):
    site = tmp_path / "site"
    site.mkdir()
    assert asset_path(site, "https://acme.com/%2e%2e/%2e%2e/evil.js") is None
    assert page_path(site, "https://acme.com/..%2f..%2fevil") is None
    assert asset_path(site, "https://acme.com/_next/static/app.js") == (site / "_next/static/app.js").resolve()


def test_pages_are_saved_as_folders_with_index_html(tmp_path):
    assert page_path(tmp_path, "https://acme.com/blog/post") == (tmp_path / "blog/post/index.html").resolve()
    assert page_path(tmp_path, "https://acme.com/about.html") == (tmp_path / "about.html").resolve()


def test_refs_finds_assets_but_leaves_page_links_to_the_crawler():
    html = ('<a href="/pricing">Pricing</a><a href="/essay.html">Essay</a>'
            '<link rel="stylesheet" href="/app.css"><link rel="icon" href="/favicon.ico?v=2">'
            '<img src="/logo.png" srcset="/logo@2x.png 2x, /logo@3x.png 3x">'
            '<div style="background:url(&quot;/bg.webp&quot;)"></div>')
    assert refs(html, BASE, "acme.com") == {
        "https://acme.com/app.css", "https://acme.com/favicon.ico?v=2", "https://acme.com/logo.png",
        "https://acme.com/logo@2x.png", "https://acme.com/logo@3x.png", "https://acme.com/bg.webp",
    }


def test_refs_ignores_data_uris_templates_and_other_hosts():
    html = ('<img srcset="data:image/png;base64,iVBOR,w0KGgo 1x">'
            '<img src="https://cdn.other.com/x.png">'
            '<script>const u="/_next/static/chunks/${id}.js", r="/static/(.*?).js"</script>')
    assert refs(html, BASE, "acme.com") == set()


def test_next_image_urls_map_to_the_original():
    html = ('<img src="/_next/image?url=%2Fhero.png&w=1920&q=75">'
            '<img src="/_next/image?url=https%3A%2F%2Fcdn.cms.io%2Fa.jpg&w=640">')
    assert refs(html, BASE, "acme.com") == {"https://acme.com/hero.png"}


def test_lazy_chunks_are_found_inside_bundles():
    js = 'e.u=function(e){return"static/chunks/"+e};var a="/_next/static/chunks/942-abc.js";'
    assert refs(js, "https://acme.com/_next/static/chunks/main.js", "acme.com") == {"https://acme.com/_next/static/chunks/942-abc.js"}


def test_runtime_assets_with_relative_paths_and_queries_are_found():
    js = ("window.__riveSrc={js:'vendor/rive/rive.js?v=e1856181',wasm:'vendor/rive/rive.wasm?v=ee9b7e35',"
          "riv:'assets/s02/read.riv?v=f5168dfe',heroimg:'assets/s01/hero-painting.webp?v=a1034a8d'};"
          "bfbin:'assets/shared/butterfly.bin?v=ae64facc';import(new URL('shaders/tear.js?v=e8e5ab2e',document.baseURI));"
          "var cdn='cdn.other.com/x.js',mod='node_modules/@acme/media/player.js';"
          '{"imports":{"./shaders/glass.js":"./shaders/glass.js?v=b45f0a07"}}')
    assert refs(js, BASE, "acme.com") == {
        "https://acme.com/vendor/rive/rive.js?v=e1856181", "https://acme.com/vendor/rive/rive.wasm?v=ee9b7e35",
        "https://acme.com/assets/s02/read.riv?v=f5168dfe", "https://acme.com/assets/s01/hero-painting.webp?v=a1034a8d",
        "https://acme.com/shaders/tear.js?v=e8e5ab2e", "https://acme.com/assets/shared/butterfly.bin?v=ae64facc",
        "https://acme.com/shaders/glass.js", "https://acme.com/shaders/glass.js?v=b45f0a07",
    }
