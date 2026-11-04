use crate::{edit, flight};
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::{path::Path, sync::LazyLock};
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
 r#"(?i)googletagmanager\.com|google-analytics\.com|\bgtag\(|\bga\(['"]|doubleclick\.net|connect\.facebook\.net|facebook\.com/tr|\bfbq\(|_linkedin_partner_id|snap\.licdn\.com|px\.ads\.linkedin\.com|static\.hotjar\.com|\bhj\(|clarity\.ms|cdn\.segment\.com|analytics\.load\(|reb2b|ddwl4m2hdecbv\.cloudfront\.net|posthog|plausible\.io|cdn\.mxpnl\.com|mixpanel|js\.hs-scripts\.com|js\.hs-analytics\.net|hs-banner|analytics\.tiktok\.com|\bttq\.|bat\.bing\.com|\buetq\b|static\.ads-twitter\.com|\btwq\(|redditstatic\.com/ads|\brdt\(|static\.cloudflareinsights\.com|data-cf-beacon|cdn\.amplitude\.com|heapanalytics|fullstory\.com|\bFS\.identify|cdn\.lr-ingest|logrocket"#
).expect("valid regex")
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<script\b[^>]*>.*?</script>|<noscript\b[^>]*>.*?</noscript>|<link\b[^>]*rel=["']?(?:preload|preconnect|dns-prefetch|prefetch)[^>]*>"#).expect("valid regex")
});
static LOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?:)?//(?:www\.)?(?:googletagmanager\.com/(?:gtm|gtag/js)|connect\.facebook\.net/[\w_]+/fbevents\.js|snap\.licdn\.com/li\.lms-analytics/insight\.min\.js|static\.hotjar\.com/c/hotjar-|(?:www\.)?clarity\.ms/tag/|cdn\.segment\.com/analytics\.js|js\.hs-scripts\.com/|static\.cloudflareinsights\.com/beacon|ddwl4m2hdecbv\.cloudfront\.net/b/|plausible\.io/js/|cdn\.amplitude\.com/|cdn\.mxpnl\.com/|analytics\.tiktok\.com/i18n/pixel|static\.ads-twitter\.com/uwt\.js|bat\.bing\.com/bat\.js)").expect("valid regex")
});
pub fn strip_html(text: &str) -> String {
    TAGS.replace_all(text, |c: &regex::Captures<'_>| {
        let tag = &c[0];
        if tag.contains("__next_f")
            || tag.contains("application/ld+json")
            || !SIGNATURE.is_match(tag)
        {
            tag.to_string()
        } else if tag.to_lowercase().starts_with("<noscript") {
            format!(
                "{}</noscript>",
                &tag[..=tag.find('>').expect("matched tag")]
            )
        } else {
            String::new()
        }
    })
    .into_owned()
}
pub fn strip_row(text: &str) -> String {
    flight::json_row(text, &|v: &mut Value| {
        if let Some(a) = v.as_array()
            && a.len() == 4
            && a[0] == "$"
            && a[3].is_object()
        {
            let props = &a[3];
            if (a[1] == "script" || props.get("strategy").is_some())
                && ["src", "id", "children", "dangerouslySetInnerHTML"]
                    .iter()
                    .any(|k| {
                        props
                            .get(k)
                            .is_some_and(|p| SIGNATURE.is_match(&p.to_string()))
                    })
            {
                *v = Value::Null;
            }
        }
    })
}
pub fn strip(site: &Path) -> Result<usize> {
    edit::apply(
        site,
        &|s| {
            LOADER
                .replace_all(&strip_row(s), |c: &regex::Captures<'_>| {
                    format!(
                        "data:text/javascript,//{}",
                        c[0].split_once("//").expect("matched URL").1
                    )
                })
                .into_owned()
        },
        &strip_html,
    )
}
