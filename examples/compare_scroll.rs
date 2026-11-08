//! Compare full viewport PNGs listed in a paired scroll-verification manifest.
//! Usage: cargo run --example compare_scroll -- states.json > comparisons.json
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Cursor};
fn pixels(path: &str) -> Result<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path)?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().context("missing image size")?];
    let info = reader.next_frame(&mut buffer)?;
    let samples = info.color_type.samples();
    ensure!(samples >= 3, "expected RGB/RGBA screenshot");
    let rgb = buffer[..info.buffer_size()]
        .chunks_exact(samples)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    Ok((info.width, info.height, rgb))
}
fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("provide states.json")?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut pairs: BTreeMap<(String, String, u64), [Option<&Value>; 2]> = BTreeMap::new();
    for row in &rows {
        let key = (
            row["profile"]
                .as_str()
                .context("missing profile")?
                .to_owned(),
            row["route"].as_str().unwrap_or("/").to_owned(),
            row["step"].as_u64().context("missing step")?,
        );
        let side = match row["side"].as_str() {
            Some("original") => 0,
            Some("generated") => 1,
            _ => anyhow::bail!("unknown side"),
        };
        ensure!(
            pairs.entry(key).or_default()[side].replace(row).is_none(),
            "duplicate comparison state"
        );
    }
    let mut results = Vec::new();
    for ((profile, route, step), pair) in pairs {
        let a = pair[0].context("missing original")?;
        let b = pair[1].context("missing generated")?;
        let (w, h, original) = pixels(a["screenshot"].as_str().context("missing screenshot")?)?;
        let (w2, h2, actual) = pixels(b["screenshot"].as_str().context("missing screenshot")?)?;
        ensure!((w, h) == (w2, h2), "image dimensions differ");
        let bad = original
            .as_chunks::<3>()
            .0
            .iter()
            .zip(actual.as_chunks::<3>().0.iter())
            .filter(|(x, y)| x.iter().zip(y.iter()).any(|(x, y)| x.abs_diff(*y) > 2))
            .count();
        let mut comparison = json!({"profile":profile,"route":route,"step":step,"y":a["y"],"pixel_mismatch":bad as f64/(f64::from(w)*f64::from(h)),"same_scroll_position":a["y"]==b["y"],"same_document_height":a["pageHeight"]==b["pageHeight"],"same_canvas_dimensions":a["canvases"]==b["canvases"],"broken_visible_images":b["brokenVisibleImages"]});
        if a.get("paths").is_some() {
            comparison["same_svg_count"] = json!(a["svgCount"] == b["svgCount"]);
            comparison["exact_svg_state"] = json!(a["paths"] == b["paths"]);
            comparison["visible_reveals_original"] = a["visibleReveals"].clone();
            comparison["visible_reveals_generated"] = b["visibleReveals"].clone();
        }
        if a.get("h1").is_some() {
            comparison["same_heading_state"] = json!(a["h1"] == b["h1"]);
            comparison["same_section_geometry"] = json!(a["sections"] == b["sections"]);
        }
        results.push(comparison);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"channel_tolerance":2,"comparisons":results}))?
    );
    Ok(())
}
