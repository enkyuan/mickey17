use mochi::cache::fingerprint;
use serde_json::{Value, json};
use std::path::Path;

pub fn element(parent: Option<usize>, tag: &str, attrs: &[(&str, &str)]) -> Value {
    json!({"parent":parent,"kind":"element","tag":tag,"namespace":"http://www.w3.org/1999/xhtml",
        "attributes":attrs.iter().map(|(name,value)|json!({"name":name,"namespace":null,"value":value})).collect::<Vec<_>>(),
        "style":0,"pseudo":{},"control":null,"bounds":{"x":0,"y":0,"width":200,"height":20}})
}
fn text(parent: usize, value: &str) -> Value {
    json!({"parent":parent,"kind":"text","value":value})
}
pub fn fixture(root: &Path, routes: usize, views: usize) {
    std::fs::create_dir_all(root.join("site")).unwrap();
    for dir in ["pages", "screens", "manifests"] {
        std::fs::create_dir_all(root.join("reconstruction").join(dir)).unwrap();
    }
    for route in 0..routes {
        let url = format!("https://example.test/catalog?kind={route}&sort=asc");
        let mut snapshots = Vec::new();
        let mut viewports = Vec::new();
        for view in 0..views {
            let height = if view == 0 { 200 } else { 240 };
            let png: &[u8] = if view == 0 {
                include_bytes!("../fixtures/source-reference.png")
            } else {
                include_bytes!("../fixtures/source-tall.png")
            };
            let snapshot = format!("reconstruction/pages/{route}-{view}.json");
            let screenshot = format!("reconstruction/screens/{route}-{view}.png");
            let value = json!({"schema_version":1,"url":url,
                "metadata":{"title":format!("Catalog {route}"),"base_url":"https://example.test/","lang":"en","doctype":"html","canonical":null,"meta":[]},
                "viewport":{"width":200,"height":height},"color_scheme":"light","state":"initial","scope":"light-dom-cssom-before-after",
                "nodes":[element(None,"html",&[]),element(Some(0),"body",&[]),
                    element(Some(1),"header",&[]),element(Some(2),"a",&[("href","/catalog?kind=1&sort=asc#items")]),text(3,"Catalog"),
                    element(Some(1),"main",&[]),
                    element(Some(5),"article",&[("class","card")]),element(Some(6),"a",&[("href",&format!("/products/{route}-a"))]),text(7,&format!("{} {route} A","Long 🙂 ".repeat(100))),element(Some(6),"img",&[("src","/image.png?size=small"),("alt","First")]),
                    element(Some(5),"article",&[("class","card featured")]),element(Some(10),"a",&[("href",&format!("/products/{route}-b"))]),text(11,&format!("Item {route} B")),element(Some(10),"img",&[("src","/image.png?size=large"),("alt","Second")]),
                    element(Some(1),"footer",&[]),element(Some(14),"p",&[]),text(15,"Shared footer")],
                "styles":[{"display":"block","font-size":if view==0 {"12px"} else {"20px"},"--brand":"#123456"}],
                "stylesheets":[{"href":null,"base_url":"https://example.test/","disabled":false,"media":"","rules":["@font-face {font-family: Fixture;src: url(/font.woff2)} @media (max-width:500px) {body {color:red}}"],"readable":true}],
                "fonts":[{"stylesheet":0,"descriptors":{"font-family":"Fixture","src":"url(/font.woff2)","font-display":"swap"}}],
                "resources":[{"url":"https://example.test/brand.svg","initiator":"img","response_status":200},{"url":"https://example.test/app.js","initiator":"script","response_status":200},{"url":"https://other.test/img.png","initiator":"img","response_status":200}],
                "screenshot":{"path":screenshot,"sha256":fingerprint(png)},"complete":true,"issues":[]});
            std::fs::write(root.join(&snapshot), serde_json::to_vec(&value).unwrap()).unwrap();
            std::fs::write(root.join(&screenshot), png).unwrap();
            snapshots.push(snapshot);
            viewports.push(value["viewport"].clone());
        }
        std::fs::write(
            root.join(format!("reconstruction/manifests/{route}.json")),
            serde_json::to_vec(
                &json!({"schema_version":1,"url":url,"viewports":viewports,"snapshots":snapshots}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    std::fs::write(root.join("site/brand.svg"), b"<svg/>").unwrap();
    std::fs::write(root.join("site/font.woff2"), b"fixture font bytes").unwrap();
    std::fs::write(root.join("site/image.png"), b"ambiguous candidate bytes").unwrap();
}
