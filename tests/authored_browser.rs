//! Generated-source integration test: real Base UI, TanStack SSR, and Bun.
#![cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "support/authored_fixture.rs"]
mod support;
use headless_chrome::{Browser, LaunchOptions, protocol::cdp::Runtime};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run(root: &Path, args: &[&str]) {
    let result = Command::new("bun")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "bun {args:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn js(tab: &headless_chrome::Tab, script: &str) -> Value {
    let params: Runtime::Evaluate = serde_json::from_value(
        json!({"expression":script,"awaitPromise":true,"returnByValue":true}),
    )
    .unwrap();
    let result = tab.call_method(params).unwrap();
    assert!(
        result.exception_details.is_none(),
        "{:?}",
        result.exception_details
    );
    result.result.value.unwrap_or(Value::Null)
}
fn wait(tab: &headless_chrome::Tab, expression: &str) {
    let start = Instant::now();
    while js(tab, expression) != true {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {expression}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
#[ignore = "requires Bun, Chrome, and dependency download"]
fn generated_direct_base_ui_behaviors_and_editable_source() {
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture");
    let mut spec = support::fixture(&capture);
    spec["components"]["CatalogPage"]["behaviors"] = json!([{"kind":"reveal","class":"page","effect":"rise","distance":12,"duration":150,"stagger":0,"threshold":0.1}]);
    spec["components"]["CatalogPage"]["body"]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({
          "kind":"primitive","name":"Switch.Root","props":{
            "aria-label":{"kind":"literal","value":"Accessibility mode"},
            "checked":{"kind":"preference","value":"accessibility_mode"},
            "onCheckedChange":{"kind":"preference","value":"set_accessibility_mode"}
          },"children":[{"kind":"primitive","name":"Switch.Thumb","children":[]}]
        }));
    spec["components"]["CatalogPage"]["styles"][0]["declarations"]["display"] = json!("grid");
    let evidence = spec["components"]["CatalogPage"]["evidence"].clone();
    spec["source_modules"] = json!({
      "features/catalog/graphics/catalog-art.tsx":{"file":"art.tsx","purpose":"Test owned module bundling","evidence":evidence},
      "assets/shaders/catalog.glsl":{"file":"catalog.glsl","purpose":"Test raw shader import","evidence":evidence}
    });
    spec["external_components"] =
        json!({"CatalogArt":{"module":"features/catalog/graphics/catalog-art.tsx","props":{}}});
    spec["components"]["CatalogPage"]["body"]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"external","name":"CatalogArt","props":{}}));
    std::fs::write(temp.path().join("art.tsx"),"import shader from '@/assets/shaders/catalog.glsl?raw'; export function CatalogArt() {return <canvas aria-label='Owned artwork' data-shader-length={shader.length} />} ").unwrap();
    std::fs::write(
        temp.path().join("catalog.glsl"),
        "void main() { gl_FragColor = vec4(1.0); }\n",
    )
    .unwrap();
    let input = temp.path().join("authored.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = temp.path().join("app");
    mochi::authored::generate(&capture, &input, &output).unwrap();
    // A real consumer edits the emitted typed content and shared token before building.
    let content = std::fs::read_dir(output.join("src/features/catalog"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-content.ts")
        })
        .unwrap();
    let text = std::fs::read_to_string(&content)
        .unwrap()
        .replace("Editable catalog", "Edited typed catalog");
    std::fs::write(content, text).unwrap();
    let tokens = output.join("src/styles/tokens.css");
    let text = std::fs::read_to_string(&tokens)
        .unwrap()
        .replace("#18232d", "#234567");
    std::fs::write(tokens, text).unwrap();
    run(&output, &["install", "--frozen-lockfile"]);
    run(&output, &["run", "build"]);
    run(&output, &["run", "check"]);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = Process(
        Command::new("bun")
            .args([".output/server/index.mjs"])
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .current_dir(&output)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    }
    let browser = Browser::new(
        LaunchOptions::default_builder()
            .path(std::env::var_os("MOCHI_CHROME").map(Into::into))
            .headless(true)
            .window_size(Some((1280, 900)))
            .build()
            .unwrap(),
    )
    .unwrap();
    let tab = browser.new_tab().unwrap();
    tab.navigate_to(&format!("http://127.0.0.1:{port}/catalog"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait(
        &tab,
        "document.querySelector('h1')?.textContent==='Edited typed catalog'",
    );
    assert_eq!(
        js(
            &tab,
            "getComputedStyle(document.querySelector('main')).color"
        ),
        "rgb(35, 69, 103)"
    );
    assert_eq!(
        js(
            &tab,
            "getComputedStyle(document.querySelector('main')).display"
        ),
        "grid"
    );
    js(
        &tab,
        "window.testErrors=[];window.addEventListener('error',e=>window.testErrors.push(e.message));true",
    );
    assert!(
        js(
            &tab,
            "Number(document.querySelector('canvas')?.dataset.shaderLength) > 20"
        ) == true
    );
    js(&tab, "document.querySelector('[role=switch]').click();true");
    wait(
        &tab,
        "document.documentElement.dataset.reducedMotion==='true' && localStorage.getItem('mochi:accessibility-mode')==='true'",
    );
    tab.reload(false, None)
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait(
        &tab,
        "document.querySelector('[role=switch]')?.getAttribute('aria-checked')==='true'",
    );
    js(
        &tab,
        "window.testErrors=[];window.addEventListener('error',e=>window.testErrors.push(e.message));true",
    );
    // Keyboard-open a menu; activate an item and verify application state.
    js(
        &tab,
        "[...document.querySelectorAll('button')].find(e=>e.textContent==='Actions').focus();true",
    );
    tab.press_key("ArrowDown").unwrap();
    wait(&tab, "!!document.querySelector('[role=menuitem]')");
    tab.press_key("Enter").unwrap();
    wait(&tab, "document.body.textContent.includes('Preview chosen')");
    // Dialog focus trap and Escape restore the trigger.
    js(
        &tab,
        "[...document.querySelectorAll('button')].find(e=>e.textContent==='Open details').click();true",
    );
    wait(&tab, "!!document.querySelector('[role=dialog]')");
    assert_eq!(
        js(
            &tab,
            "document.querySelector('[role=dialog]').getAttribute('aria-labelledby') !== null"
        ),
        true
    );
    tab.press_key("Escape").unwrap();
    wait(&tab, "!document.querySelector('[role=dialog]')");
    wait(&tab, "document.activeElement?.textContent==='Open details'");
    // Navigation opens through Base UI; the link remains a real router anchor.
    js(
        &tab,
        "[...document.querySelectorAll('button')].find(e=>e.textContent==='Explore').click();true",
    );
    wait(
        &tab,
        "[...document.querySelectorAll('a')].some(e=>e.textContent==='Catalog' && e.getAttribute('href')==='/catalog')",
    );
    tab.press_key("Escape").unwrap();
    // Label association, native invalid email refusal, then local valid submit.
    assert_eq!(
        js(
            &tab,
            "document.querySelector('label').control===document.querySelector('input')"
        ),
        true
    );
    js(&tab, "document.querySelector('form').requestSubmit();true");
    assert!(
        !js(
            &tab,
            "document.body.textContent.includes('Locally submitted')"
        )
        .as_bool()
        .unwrap()
    );
    tab.find_element("input")
        .unwrap()
        .click()
        .unwrap()
        .type_into("test@example.test")
        .unwrap();
    js(&tab, "document.querySelector('form').requestSubmit();true");
    wait(
        &tab,
        "document.body.textContent.includes('Locally submitted')",
    );
    assert_eq!(js(&tab, "window.testErrors"), json!([]));
    assert_eq!(
        js(
            &tab,
            "[...document.scripts].some(s=>s.src.includes('recovered'))"
        ),
        false
    );
}
