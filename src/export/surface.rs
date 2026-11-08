use super::*;
use crate::evidence::SurfaceKind;
impl Renderer<'_> {
    pub(super) fn surface(&mut self, source: &SourceRef) -> Result<Option<String>> {
        let capture = &self.model.observations[&source.observation].capture;
        let Some(surface) = capture.surfaces.get(&source.node) else {
            return Ok(None);
        };
        let Content::Element {
            tag, attributes, ..
        } = &capture.nodes[source.node].content
        else {
            anyhow::bail!("surface is not an element")
        };
        ensure!(
            tag.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && !matches!(
                    tag.as_str(),
                    "script"
                        | "style"
                        | "link"
                        | "meta"
                        | "base"
                        | "object"
                        | "embed"
                        | "applet"
                        | "template"
                ),
            "invalid surface host"
        );
        if surface.kind == SurfaceKind::Raster {
            ensure!(
                matches!(tag.as_str(), "canvas" | "video" | "iframe" | "frame"),
                "invalid raster host"
            );
        }
        if surface.kind == SurfaceKind::Frame {
            ensure!(
                matches!(tag.as_str(), "iframe" | "frame"),
                "invalid frame host"
            );
        }
        let mut raw: BTreeMap<String, String> = attributes
            .iter()
            .filter(|a| {
                !a.name.to_ascii_lowercase().starts_with("on")
                    && !matches!(
                        a.name.as_str(),
                        "src" | "srcdoc" | "sandbox" | "action" | "formaction"
                    )
            })
            .map(|a| (a.name.clone(), a.value.clone()))
            .collect();
        let observed = observed_style(capture, source.node);
        if !observed.is_empty() {
            let style = raw.entry("style".into()).or_default();
            style.push(';');
            style.push_str(&observed);
        }
        let raw = js(&raw)?;
        match surface.kind {
            SurfaceKind::Nonvisual => {
                self.report
                    .note("nonvisual_runtime_surface_excluded", Some(source.clone()));
                Ok(Some("{null}".into()))
            }
            SurfaceKind::Raster => {
                let image = self
                    .surface_images
                    .get(&(source.observation.clone(), source.node))
                    .context("surface image missing")?;
                self.report
                    .note("editable_static_surface_snapshot", Some(source.clone()));
                Ok(Some(format!(
                    "<CapturedSurface tag={{{}}} raw={{{raw}}} image={{{}}} />",
                    js(tag)?,
                    js(image)?
                )))
            }
            SurfaceKind::Frame => {
                let base = Url::parse(&surface.base_url)?;
                let mut stylesheet = String::new();
                for rule in &surface.css {
                    stylesheet.push_str(&css(rule, &base, &self.asset_urls, &mut self.report)?);
                    stylesheet.push('\n');
                }
                let mut html = String::new();
                for &node in &surface.children {
                    html.push_str(&self.static_html(
                        &SourceRef {
                            observation: source.observation.clone(),
                            node,
                        },
                        &base,
                    )?);
                }
                // A sandbox without capabilities disables scripts, forms, popups and top navigation.
                let document = format!(
                    "<!doctype html><meta charset=\"utf-8\"><style>{}</style>{html}",
                    Regex::new("(?i)</style")
                        .expect("style close pattern")
                        .replace_all(&stylesheet, "\\3c /style")
                );
                self.report
                    .note("sandboxed_frame_source", Some(source.clone()));
                Ok(Some(format!(
                    "<iframe {{...elementProps('iframe',{raw},null,base)}} sandbox=\"\" srcDoc={{{}}} />",
                    js(&document)?
                )))
            }
            SurfaceKind::Shadow => {
                let base = Url::parse(&surface.base_url)?;
                let mut stylesheet = String::new();
                for rule in &surface.css {
                    stylesheet.push_str(&css(rule, &base, &self.asset_urls, &mut self.report)?);
                    stylesheet.push('\n');
                }
                let children = self.trees[&source.observation][source.node].clone();
                let mut light = String::new();
                let mut shadow = String::new();
                for node in children {
                    let jsx = self.node(
                        &SourceRef {
                            observation: source.observation.clone(),
                            node,
                        },
                        &BTreeMap::new(),
                        None,
                    )?;
                    if surface.children.contains(&node) {
                        shadow.push_str(&jsx)
                    } else {
                        light.push_str(&jsx)
                    }
                }
                self.report
                    .note("shadow_dom_source_preserved", Some(source.clone()));
                Ok(Some(format!(
                    "<ShadowHost tag={{{}}} raw={{{raw}}} mode={{{}}} css={{{}}} light={{<>{light}</>}}>{shadow}</ShadowHost>",
                    js(tag)?,
                    js(&surface.mode)?,
                    js(&stylesheet)?
                )))
            }
        }
    }
    fn static_html(&mut self, source: &SourceRef, base: &Url) -> Result<String> {
        let capture = &self.model.observations[&source.observation].capture;
        match &capture.nodes[source.node].content {
            Content::Text { value } => Ok(html_escape::encode_text(value).into_owned()),
            Content::Element {
                tag,
                attributes,
                control,
                ..
            } => {
                if matches!(
                    tag.as_str(),
                    "script"
                        | "style"
                        | "link"
                        | "meta"
                        | "base"
                        | "object"
                        | "embed"
                        | "applet"
                        | "template"
                ) {
                    return Ok(String::new());
                }
                ensure!(
                    tag.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                    "invalid frame tag"
                );
                if let Some(surface) = capture.surfaces.get(&source.node) {
                    if surface.kind == SurfaceKind::Nonvisual {
                        return Ok(String::new());
                    }
                    if surface.kind == SurfaceKind::Raster {
                        let image = self
                            .surface_images
                            .get(&(source.observation.clone(), source.node))
                            .context("nested surface image missing")?;
                        return Ok(format!(
                            "<img src=\"{image}\" style=\"width:{}px;height:{}px\">",
                            surface.bounds.width, surface.bounds.height
                        ));
                    }
                    ensure!(
                        surface.kind != SurfaceKind::Frame && surface.kind != SurfaceKind::Shadow,
                        "nested frame/shadow serialization requires its own boundary"
                    );
                }
                let mut out = format!("<{tag}");
                let observed = observed_style(capture, source.node);
                for a in attributes {
                    if control.is_some()
                        && ((matches!(tag.as_str(), "input" | "textarea") && a.name == "value")
                            || (tag == "input" && a.name == "checked")
                            || (tag == "option" && a.name == "selected")
                            || (tag == "details" && a.name == "open"))
                    {
                        continue;
                    }
                    if a.name.to_ascii_lowercase().starts_with("on")
                        || matches!(
                            a.name.as_str(),
                            "srcdoc"
                                | "action"
                                | "formaction"
                                | "nonce"
                                | "integrity"
                                | "is"
                                | "autoplay"
                        )
                    {
                        continue;
                    }
                    ensure!(
                        a.name
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c)),
                        "invalid frame attribute"
                    );
                    let value =
                        if matches!(a.name.as_str(), "href" | "src" | "poster" | "xlink:href") {
                            let url = base.join(&a.value)?;
                            if !matches!(url.scheme(), "http" | "https" | "mailto" | "tel")
                                && !a.value.starts_with("data:image/")
                            {
                                continue;
                            }
                            self.asset_urls
                                .get(url.as_str())
                                .cloned()
                                .unwrap_or_else(|| url.to_string())
                        } else if a.name == "style" {
                            css(
                                &format!("{};{}", a.value, observed),
                                base,
                                &self.asset_urls,
                                &mut self.report,
                            )?
                        } else {
                            a.value.clone()
                        };
                    out.push_str(&format!(
                        " {}=\"{}\"",
                        a.name,
                        html_escape::encode_double_quoted_attribute(&value)
                    ));
                }
                if !observed.is_empty() && !attributes.iter().any(|a| a.name == "style") {
                    out.push_str(&format!(
                        " style=\"{}\"",
                        html_escape::encode_double_quoted_attribute(&observed)
                    ));
                }
                if let Some(c) = control
                    && let Some(value) = &c.value
                    && tag == "input"
                {
                    out.push_str(&format!(
                        " value=\"{}\"",
                        html_escape::encode_double_quoted_attribute(value)
                    ));
                }
                out.push('>');
                if !matches!(
                    tag.as_str(),
                    "area"
                        | "base"
                        | "br"
                        | "col"
                        | "embed"
                        | "hr"
                        | "img"
                        | "input"
                        | "link"
                        | "meta"
                        | "param"
                        | "source"
                        | "track"
                        | "wbr"
                ) {
                    for node in self.trees[&source.observation][source.node].clone() {
                        out.push_str(&self.static_html(
                            &SourceRef {
                                observation: source.observation.clone(),
                                node,
                            },
                            base,
                        )?);
                    }
                    out.push_str(&format!("</{tag}>"));
                }
                Ok(out)
            }
        }
    }
}
