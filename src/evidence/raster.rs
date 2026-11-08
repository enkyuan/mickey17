use super::*;
/// Isolate a tainted canvas or opaque frame before a browser-side raster capture.
/// Original inline styles are restored even when screenshot creation fails.
pub(super) fn isolated(tab: &Tab, node: usize, b: &Bounds) -> Result<Vec<u8>> {
    tab.evaluate(&format!(r#"(()=>{{const node=window.__mochiCapturedNodes[{node}];const styles=[],saved=[];const hide=root=>{{const s=(root.ownerDocument||root).createElement('style');s.textContent='*{{visibility:hidden!important}}';(root.head||root).appendChild(s);styles.push(s)}};hide(document);if(node.ownerDocument!==document)hide(node.ownerDocument);let current=node;while(current){{saved.push([current,current.getAttribute('style')]);current.style.setProperty('visibility','visible','important');const root=current.getRootNode();if(root.host){{hide(root);current=root.host}}else{{current=current.ownerDocument.defaultView.frameElement}}}};window.__mochiRestoreRaster=()=>{{for(const s of styles)s.remove();for(const [n,value] of saved){{if(value===null)n.removeAttribute('style');else n.setAttribute('style',value)}}}};return true}})()"#),false)?;
    let result = (|| {
        let background: Emulation::SetDefaultBackgroundColorOverride =
            serde_json::from_value(serde_json::json!({"color":{"r":0,"g":0,"b":0,"a":0}}))?;
        tab.call_method(background)?;
        tab.capture_screenshot(
            Page::CaptureScreenshotFormatOption::Png,
            None,
            Some(Page::Viewport {
                x: b.x.max(0.0),
                y: b.y.max(0.0),
                width: b.width,
                height: b.height,
                scale: 1.0,
            }),
            true,
        )
    })();
    let restore = tab.evaluate(
        "(()=>{window.__mochiRestoreRaster?.();delete window.__mochiRestoreRaster;return true})()",
        false,
    );
    let background = tab.call_method(Emulation::SetDefaultBackgroundColorOverride { color: None });
    let png = result?;
    restore?;
    background?;
    Ok(png)
}
