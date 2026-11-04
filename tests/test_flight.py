from nextpage import decoded, page_with

from perfectclone.flight import edit_flight, map_rows, outside_flight


def test_text_row_length_is_recomputed_in_utf8_bytes():
    text = "Acme — the platform"  # the em dash is 3 bytes
    stream = f'0:["$","h1",null,{{"children":"Acme"}}]\n1a:T{len(text.encode()):x},{text}2:null\n'
    new = map_rows(stream, lambda s: s.replace("Acme", "Globex"))
    replaced = "Globex — the platform"
    assert f"1a:T{len(replaced.encode()):x},{replaced}2:null\n" in new
    assert new.startswith('0:["$","h1",null,{"children":"Globex"}]\n')


def test_rows_split_across_chunks_are_edited():
    stream = '0:["$","p",null,{"children":"Hello Acme, welcome to Acme"}]\n'
    new = edit_flight(page_with(stream, chunks=3), lambda s: s.replace("Acme", "Globex"))
    assert decoded(new) == stream.replace("Acme", "Globex")


def test_outside_flight_leaves_payload_alone():
    new = outside_flight(page_with('0:"Acme"\n'), lambda s: s.replace("Acme", "X"))
    assert "<h1>X</h1>" in new and decoded(new) == '0:"Acme"\n'


def test_unchanged_page_is_returned_as_is():
    page = page_with('0:"Acme"\n', chunks=2)
    assert edit_flight(page, lambda s: s) is page
