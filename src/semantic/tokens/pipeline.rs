//! End-to-end run: capture directory -> token set -> central styles file, rewritten
//! stylesheets, manifest and report, with static equality checks against the capture.

use super::capture::Corpus;
use super::css::{self, Sheet};
use super::emit;
use super::inventory::Inventory;
use super::plan::{Options, TokenSet};
use super::rewrite::{self, RewriteStats};
use super::value::{Category, Context};
use super::verify::{self, ResolverCheck, RewriteCheck};
use anyhow::{Context as _, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const HARNESS: &str = include_str!("../../../scripts/tokens/computed-equality.js");

#[derive(Debug, Serialize)]
pub struct SnapshotInfo {
    pub label: String,
    pub file: String,
    pub sha256: String,
    pub url: String,
    pub viewport: [u32; 2],
    pub color_scheme: String,
    pub root_font_size_px: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct SheetInfo {
    pub index: usize,
    /// `<link media>` condition now written as an @media wrapper in the output file.
    pub media: Option<String>,
    pub href: Option<String>,
    pub sha256: String,
    pub css_bytes: usize,
    pub from_text_source: bool,
    /// Rules the CSS parser could not interpret; kept verbatim in the output.
    pub unparsed_nodes: usize,
    pub output: String,
    pub rewrite: RewriteStats,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub tokens: usize,
    pub by_category: BTreeMap<Category, usize>,
    pub by_origin: BTreeMap<String, usize>,
    pub by_basis: BTreeMap<String, usize>,
    pub hoisted_site_variables: usize,
    pub site_variables_kept_in_place: usize,
    pub unnamed_eligible_values: usize,
    pub single_use_literals: BTreeMap<Category, usize>,
    pub declarations_scanned: usize,
    pub replacements: usize,
    pub merged_across_px_rem: usize,
    /// Declarations converted between px and rem by the spacing scale.
    pub spacing_converted_between_px_and_rem: usize,
    /// Property comparisons whose rewritten winner references (or defines) a token, summed over snapshots.
    pub rewritten_comparisons: usize,
    pub total_comparisons: usize,
    pub derived_tokens_exercised: usize,
    pub derived_tokens_unexercised: usize,
    /// Cascade-vs-capture mismatches of the resolver itself (see resolver_validation).
    pub resolver_mismatches: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub tool: &'static str,
    pub status: &'static str,
    pub capture: String,
    pub options: OptionsInfo,
    pub snapshots: Vec<SnapshotInfo>,
    pub sheets: Vec<SheetInfo>,
    pub constant_root_font_size_px: Option<f64>,
    pub summary: Summary,
    pub tokens: Vec<super::plan::Token>,
    pub spacing: Option<super::plan::SpacingScale>,
    pub breakpoints: Vec<super::plan::Breakpoint>,
    pub unnamed: Vec<super::plan::Unnamed>,
    pub hoisted: Vec<super::plan::Hoisted>,
    pub kept_vars: Vec<super::plan::KeptVar>,
    pub resolver_validation: Vec<ResolverCheck>,
    pub rewrite_equality: Vec<RewriteCheck>,
    /// Derived tokens (and the spacing base) that no compared element references: their
    /// replacements are not backed by any equality comparison.
    pub unexercised_tokens: Vec<String>,
    pub media_assumptions: Vec<&'static str>,
    pub gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct OptionsInfo {
    pub min_uses: usize,
    pub merge_rem_px: bool,
}

pub struct Output {
    /// Relative path -> file content.
    pub files: BTreeMap<String, String>,
    pub report: Report,
    /// The decided tokens, for emitters that need `token_for` / `class_for`.
    pub tokens: TokenSet,
}

/// Bundler content hashes are 8 base64url characters after a dash (`styles-B8ae2haK`,
/// `features-BWKZ-Z7S`) with a digit or mixed case so ordinary words are never stripped.
fn hashy(part: &str) -> bool {
    part.len() == 8
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && (part.chars().any(|c| c.is_ascii_digit())
            || (part.chars().any(|c| c.is_ascii_uppercase())
                && part.chars().any(|c| c.is_ascii_lowercase())))
}

fn strip_hash(stem: &str) -> &str {
    if stem.len() > 9 && stem.is_char_boundary(stem.len() - 9) {
        let (head, tail) = stem.split_at(stem.len() - 9);
        if let Some(hash) = tail.strip_prefix('-')
            && hashy(hash)
        {
            return head;
        }
    }
    stem
}

/// A readable file name for a stylesheet: the URL stem without bundler hashes.
fn sheet_name(href: Option<&str>, taken: &mut BTreeSet<String>) -> String {
    let (stem, host) = match href.and_then(|h| url::Url::parse(h).ok()) {
        Some(u) => {
            let last = u
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or("")
                .to_string();
            (last, u.host_str().unwrap_or("").to_string())
        }
        None => (String::new(), String::new()),
    };
    let stem = stem.strip_suffix(".css").unwrap_or(&stem).to_string();
    let mut base: String = strip_hash(&stem).to_string();
    if base.is_empty() {
        base = if href.is_none() {
            "inline".into()
        } else {
            "styles".into()
        };
    }
    let base: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if taken.insert(base.clone()) {
        return base;
    }
    let host_slug: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let with_host = format!("{host_slug}-{base}");
    if !host_slug.is_empty() && taken.insert(with_host.clone()) {
        return with_host;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// A `<link media>` value we can carry into an `@media` prelude unchanged: it must parse as
/// exactly one media rule whose prelude is the same text, with no block, statement or at-rule
/// smuggled in.
fn valid_media(media: &str) -> bool {
    if media.contains(['{', '}', ';', '@']) {
        return false;
    }
    let parsed = css::parse_sheet(&format!("@media {media} {{}}"));
    matches!(parsed.nodes.as_slice(), [css::Node::At(at)] if at.name == "media" && at.raw_prelude.as_deref() == Some(media.trim()))
}

/// Name an inline stylesheet after the component its first rule styles.
fn inline_sheet_name(sheet: &Sheet, taken: &mut BTreeSet<String>) -> String {
    let subject = sheet.nodes.iter().find_map(|n| match n {
        css::Node::Style(rule) => super::inventory::subject_of(&rule.selectors),
        _ => None,
    });
    let base = match subject {
        Some(s) if s != "html" && s != "body" => format!("inline-{s}"),
        _ => "inline".to_string(),
    };
    if taken.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

pub fn run(capture_dir: &Path, options: &Options) -> Result<Output> {
    let corpus = Corpus::load(capture_dir)?;
    let sheets = verify::parse_corpus(&corpus);

    // Sheets some snapshot actually applies (disabled sheets never count) and the media
    // condition they were linked with.
    let used: BTreeSet<usize> = corpus
        .snapshots
        .iter()
        .flat_map(|s| s.sheets.iter().filter(|r| !r.disabled).map(|r| r.sheet))
        .collect();
    let mut media_gaps: Vec<String> = Vec::new();
    let sheet_media: BTreeMap<usize, Option<String>> = used
        .iter()
        .map(|&i| {
            let mut conditions: BTreeSet<String> = BTreeSet::new();
            for s in &corpus.snapshots {
                for r in s.sheets.iter().filter(|r| !r.disabled && r.sheet == i) {
                    let m = r.media.trim();
                    conditions.insert(if m.eq_ignore_ascii_case("all") { String::new() } else { m.to_string() });
                }
            }
            if conditions.len() > 1 {
                media_gaps.push(format!(
                    "Stylesheet #{i} is linked with different media conditions in different snapshots ({conditions:?}); the first non-empty one is kept as its @media wrapper."
                ));
            }
            (i, conditions.into_iter().find(|c| !c.is_empty()))
        })
        .collect();
    for (index, media) in &sheet_media {
        if let Some(media) = media {
            ensure!(
                valid_media(media),
                "stylesheet #{index} has an unsupported <link media> value: {media:?}"
            );
        }
    }
    let pairs: Vec<(&Sheet, bool)> = used
        .iter()
        .map(|&i| (&sheets[i], sheet_media[&i].is_some()))
        .collect();

    // Root font size: rem merges with px only when it is the same in every snapshot and no
    // rule changes the root size under a condition (a viewport no snapshot visited).
    let roots: Vec<Option<f64>> = corpus
        .snapshots
        .iter()
        .map(|s| s.root_font_size_px())
        .collect();
    let constant_root = match roots.first() {
        Some(Some(first)) if roots.iter().all(|r| *r == Some(*first)) => Some(*first),
        _ => None,
    };
    let conditional_root = super::inventory::root_font_size_is_conditional(&pairs);
    let ctx = Context {
        rem_px: if options.merge_rem_px && !conditional_root {
            constant_root
        } else {
            None
        },
    };

    // Inventory over every distinct sheet that some snapshot uses.
    let mut inventory = Inventory::collect_with(&pairs, ctx);
    // Custom properties set by inline style attributes are reserved names too.
    for snapshot in &corpus.snapshots {
        for node in snapshot.elements() {
            if let Some(style) = super::selector::Dom::attr(snapshot, node, "style") {
                for decl in css::parse_declarations(style)
                    .into_iter()
                    .filter(|d| d.is_custom())
                {
                    inventory.vars.entry(decl.name).or_default().inline_set = true;
                }
            }
        }
    }
    let cascades: Vec<_> = corpus
        .snapshots
        .iter()
        .map(|s| verify::cascade_for(s, &sheets))
        .collect();
    for (snapshot, cascade) in corpus.snapshots.iter().zip(&cascades) {
        let elements = snapshot.elements().map(|node| {
            let element = snapshot.element(node).expect("element");
            let mut pseudos: Vec<Option<String>> = vec![None];
            pseudos.extend(element.pseudo.keys().map(|k| Some(k.clone())));
            (node, pseudos)
        });
        inventory.add_weights(snapshot, elements, cascade, ctx);
    }
    let set = TokenSet::build(&inventory, ctx, options);

    // Central file.
    let mut taken = BTreeSet::new();
    // URLs are untrusted text inside a CSS comment: neutralize anything that could end it.
    let comment_safe = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .replace("*/", "*\\/")
            .replace("/*", "/\\*")
    };
    let source_names: Vec<String> = used
        .iter()
        .map(|&i| {
            comment_safe(
                &corpus.sheets[i]
                    .href
                    .clone()
                    .unwrap_or_else(|| format!("inline stylesheet #{i}")),
            )
        })
        .collect();
    let central = emit::central_css(&set, &source_names);
    let central_sheet = css::parse_sheet(&central);
    ensure!(
        css::raw_count(&central_sheet.nodes) == 0,
        "the generated central file did not parse back cleanly"
    );

    // Rewrite every distinct sheet.
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let mut sheet_infos = Vec::new();
    for &index in &used {
        // Sheets read from raw text (no CSSOM validation) keep their values: a declaration the
        // browser would have rejected must not become valid by being rewritten.
        let replace_values = !corpus.sheets[index].from_text_source;
        let (mut sheet, stats) = rewrite::rewrite_sheet(&sheets[index], &set, ctx, replace_values);
        if let Some(media) = &sheet_media[&index] {
            // `<link media>` becomes an explicit @media wrapper so the condition survives.
            // @charset/@namespace are not valid inside @media and apply document-wide anyway, so
            // they stay in front of it; an @import would lose its condition, which is refused.
            let (front, inner): (Vec<css::Node>, Vec<css::Node>) = std::mem::take(&mut sheet.nodes)
                .into_iter()
                .partition(|n| matches!(n, css::Node::At(a) if matches!(a.name.as_str(), "charset" | "namespace")));
            ensure!(
                !inner
                    .iter()
                    .any(|n| matches!(n, css::Node::At(a) if a.name == "import")),
                "stylesheet #{index} has a <link media> condition and an @import; that cannot be expressed without changing the import"
            );
            let items = inner.into_iter().map(css::Item::Rule).collect();
            sheet.nodes = front;
            sheet.nodes.push(css::Node::At(css::AtRule {
                name: "media".into(),
                prelude: css::parse_components(media),
                raw_prelude: Some(media.clone()),
                items: Some(items),
            }));
        }
        let name = match corpus.sheets[index].href.as_deref() {
            Some(href) => sheet_name(Some(href), &mut taken),
            None => inline_sheet_name(&sheets[index], &mut taken),
        };
        let output = format!("src/styles/rewritten/{name}.css");
        files.insert(output.clone(), sheet.to_css());
        sheet_infos.push(SheetInfo {
            media: sheet_media[&index].clone(),
            index,
            href: corpus.sheets[index].href.clone(),
            sha256: corpus.sheets[index].sha256.clone(),
            css_bytes: corpus.sheets[index].texts.iter().map(String::len).sum(),
            from_text_source: corpus.sheets[index].from_text_source,
            unparsed_nodes: css::raw_count(&sheets[index].nodes),
            output,
            rewrite: stats,
        });
    }
    files.insert("src/styles/tokens.css".into(), central.clone());

    // Static checks against the capture. The "after" cascade is built by re-parsing the files
    // exactly as they will be written, so serialization defects cannot hide.
    let tokens_plain = verify::theme_as_root_sheet(&central_sheet);
    let definitions = verify::theme_definitions(&central_sheet);
    // Every variable the central file defines may appear on pages whose own stylesheets never
    // defined it (the file is shared by the whole site); where both sides have a winner the
    // values are still compared.
    let introduced: BTreeSet<String> = definitions.keys().cloned().collect();
    let mut after_sheets: Vec<Sheet> = vec![Sheet::default(); sheets.len()];
    for info in &sheet_infos {
        let reparsed = css::parse_sheet(&files[&info.output]);
        ensure!(
            css::raw_count(&reparsed.nodes) <= css::raw_count(&sheets[info.index].nodes),
            "{} did not parse back cleanly (more unparsable nodes than the source)",
            info.output
        );
        after_sheets[info.index] = reparsed;
    }
    after_sheets.push(tokens_plain);
    let tokens_index = after_sheets.len() - 1;

    let mut resolver_validation = Vec::new();
    let mut rewrite_equality = Vec::new();
    for (snapshot, before) in corpus.snapshots.iter().zip(&cascades) {
        let mut check = verify::validate_resolver(snapshot, before);
        check.samples.truncate(10);
        resolver_validation.push(check);

        let mut inputs = vec![super::cascade::SheetInput {
            sheet: &after_sheets[tokens_index],
            index: tokens_index,
            media: "",
        }];
        // The media condition now lives inside the rewritten file, so the input carries none.
        inputs.extend(snapshot.sheets.iter().filter(|s| !s.disabled).map(|s| {
            super::cascade::SheetInput {
                sheet: &after_sheets[s.sheet],
                index: s.sheet,
                media: "",
            }
        }));
        let after = super::cascade::Cascade::build(&inputs, &verify::environment(snapshot));
        let mut check = verify::verify_rewrite(snapshot, before, &after, &introduced);
        check.samples.truncate(20);
        rewrite_equality.push(check);
    }
    let mut exercised: BTreeMap<String, usize> = BTreeMap::new();
    for c in &rewrite_equality {
        for (name, n) in &c.exercised {
            *exercised.entry(name.clone()).or_default() += n;
        }
    }
    let unexercised: Vec<String> = set
        .tokens
        .iter()
        .filter(|t| t.origin == super::plan::Origin::Derived)
        .map(|t| t.name.clone())
        .chain(set.spacing.iter().map(|s| s.name.clone()))
        .filter(|n| !exercised.contains_key(n))
        .collect();
    let any_difference = rewrite_equality.iter().any(|c| c.different > 0);

    let manifest = emit::manifest(&set, "src/styles/tokens.css");
    files.insert(
        "tokens.manifest.json".into(),
        serde_json::to_string_pretty(&manifest)?,
    );
    files.insert("tokens.equality-harness.js".into(), HARNESS.to_string());

    let mut by_category: BTreeMap<Category, usize> = BTreeMap::new();
    let mut by_origin: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_basis: BTreeMap<String, usize> = BTreeMap::new();
    for t in &set.tokens {
        *by_category.entry(t.category).or_default() += 1;
        *by_origin
            .entry(format!("{:?}", t.origin).to_lowercase())
            .or_default() += 1;
        *by_basis.entry(t.basis.label().to_string()).or_default() += 1;
    }
    let summary = Summary {
        tokens: set.tokens.len(),
        by_category,
        by_origin,
        by_basis,
        hoisted_site_variables: set.hoisted.len(),
        site_variables_kept_in_place: set.kept_vars.len(),
        unnamed_eligible_values: set.unnamed.len(),
        single_use_literals: set.single_use.clone(),
        declarations_scanned: inventory.declarations,
        replacements: sheet_infos.iter().map(|s| s.rewrite.replacements).sum(),
        merged_across_px_rem: set.tokens.iter().filter(|t| t.merged_across_units).count(),
        spacing_converted_between_px_and_rem: set
            .spacing
            .as_ref()
            .map_or(0, |s| s.converted_between_px_and_rem),
        rewritten_comparisons: rewrite_equality.iter().map(|c| c.rewritten_compared).sum(),
        total_comparisons: rewrite_equality.iter().map(|c| c.compared).sum(),
        derived_tokens_exercised: set
            .tokens
            .iter()
            .filter(|t| t.origin == super::plan::Origin::Derived)
            .count()
            .saturating_sub(
                unexercised
                    .iter()
                    .filter(|n| n.as_str() != "--spacing")
                    .count(),
            ),
        derived_tokens_unexercised: unexercised.len(),
        resolver_mismatches: resolver_validation.iter().map(|c| c.mismatched).sum(),
    };
    let snapshots = corpus
        .snapshots
        .iter()
        .map(|s| SnapshotInfo {
            label: s.label.clone(),
            file: s.file.display().to_string(),
            sha256: s.sha256.clone(),
            url: s.url.clone(),
            viewport: [s.width, s.height],
            color_scheme: s.color_scheme.clone(),
            root_font_size_px: s.root_font_size_px(),
        })
        .collect();
    let schemes: BTreeSet<&str> = corpus
        .snapshots
        .iter()
        .map(|s| s.color_scheme.as_str())
        .collect();
    let mut gaps = vec![
        format!(
            "Only the {:?} color scheme was captured: values that exist only under another prefers-color-scheme branch are not verified.",
            schemes
        ),
        "Snapshots are resting states. :hover, :focus, :active, :visited and :target are treated as not matching, so rewritten declarations inside such rules (and in rules for elements absent from every snapshot) are never compared; see unexercised_tokens for tokens no compared element references.".into(),
        "Derived tokens in Tailwind namespaces (--color-*, --spacing, --radius-*, --text-*) override Tailwind's defaults of the same name inside @theme: the generated project's theme is the site's theme.".into(),
        "Rewrite equality compares sRGB colors by 8-bit channels and alpha, lengths, line-height and opacity to four decimals, oklab/oklch to three and other numbers to six; no layout rounding is applied. Merging uses the same precision, so differences below it are invisible to the check.".into(),
        "Equality is checked statically against the captured DOM and computed styles; run tokens.equality-harness.js in a browser against the original and rewritten projects for runtime evidence.".into(),
        "Media-query breakpoints stay literal (custom properties are not valid in media conditions); breakpoint tokens are documentation/Tailwind inputs only.".into(),
        "Lengths inside functions (calc(), clamp(), min(), max()), percentages and viewport units are left as written.".into(),
        "Declarations in inline style attributes, JavaScript-set styles, @font-face descriptors and @property registrations are not rewritten.".into(),
        "Shadow DOM and iframe surfaces use their own stylesheets and are not covered.".into(),
        "Conditions that cannot be decided from a capture (@container, @scope, vendor @supports, unknown pseudo-classes) are treated as undecided and excluded from equality counts.".into(),
    ];
    if constant_root.is_none() {
        gaps.push("The root font size differs between snapshots, so rem lengths were not merged with px lengths.".into());
    }
    if conditional_root {
        gaps.push("A rule changes the root element's font size under a media/supports/container condition, so rem lengths were not merged with px lengths (a viewport no snapshot visited could compute them differently).".into());
    }
    if let Some(scale) = &set.spacing
        && scale.converted_between_px_and_rem > 0
    {
        gaps.push(format!(
                "{} spacing declarations were written in a different unit (px/rem) than the scale's and were converted; they are computed-equal at the captured root font size but would differ if the user changes the browser font size.",
                scale.converted_between_px_and_rem
            ));
    }
    for &index in &used {
        if corpus.sheets[index].from_text_source {
            gaps.push(format!(
                "Stylesheet #{index} was captured as raw text, not CSSOM rules, so its values are not rewritten (invalid declarations cannot be told from valid ones); only its hoisted variable definitions moved."
            ));
        }
    }
    for info in &sheet_infos {
        if info.unparsed_nodes > 0 {
            gaps.push(format!(
                "{}: {} rule(s) could not be parsed (for example unsupported selector syntax); they are copied unchanged and take part in no token decision.",
                info.output, info.unparsed_nodes
            ));
        }
    }
    gaps.push("Resolution and verification slow down super-linearly on very deep or very wide element trees (thousands of levels or siblings); the captured sites finish in seconds.".into());
    gaps.extend(set.gaps.iter().cloned());
    gaps.extend(media_gaps);
    let report = Report {
        schema_version: 1,
        tool: "mochi tokens",
        status: if any_difference {
            "static_differences_found"
        } else if rewrite_equality.iter().any(|c| c.compared == 0) {
            "static_check_vacuous"
        } else {
            "static_checks_only_not_browser_verified"
        },
        capture: capture_dir.display().to_string(),
        options: OptionsInfo {
            min_uses: options.min_uses,
            merge_rem_px: options.merge_rem_px,
        },
        snapshots,
        sheets: sheet_infos,
        constant_root_font_size_px: constant_root,
        summary,
        tokens: set.tokens.clone(),
        spacing: set.spacing.clone(),
        breakpoints: set.breakpoints.clone(),
        unnamed: set.unnamed.clone(),
        hoisted: set.hoisted.clone(),
        kept_vars: set.kept_vars.clone(),
        resolver_validation,
        rewrite_equality,
        unexercised_tokens: unexercised,
        media_assumptions: super::media::Environment::assumptions(),
        gaps,
    };
    files.insert(
        "tokens.report.json".into(),
        serde_json::to_string_pretty(&report)?,
    );
    files.insert("tokens.report.md".into(), markdown(&report));
    Ok(Output {
        files,
        report,
        tokens: set,
    })
}

fn markdown(r: &Report) -> String {
    let mut out = String::new();
    out.push_str("# Token report\n\n");
    out.push_str(&format!(
        "Status: `{}`. Capture: `{}`.\n\n",
        r.status, r.capture
    ));
    let s = &r.summary;
    out.push_str(&format!(
        "{} tokens ({}), {} site variables hoisted into the central file, {} left in place, {} eligible values without a defensible name, {} replacements in {} declarations scanned.\n\n",
        s.tokens,
        s.by_origin.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", "),
        s.hoisted_site_variables,
        s.site_variables_kept_in_place,
        s.unnamed_eligible_values,
        s.replacements,
        s.declarations_scanned,
    ));
    out.push_str("## Tokens by category\n\n| Category | Tokens |\n| --- | ---: |\n");
    for (c, n) in &s.by_category {
        out.push_str(&format!("| {} | {n} |\n", c.label()));
    }
    if let Some(scale) = &r.spacing {
        out.push_str(&format!(
            "\n## Spacing scale\n\n`{}: {}` ({}), {} declarations on the scale, {} off-scale left literal.\n",
            scale.name,
            scale.value,
            if scale.site_defined { "site-defined" } else { "derived" },
            scale.members,
            scale.off_scale
        ));
    }
    out.push_str("\n## Tokens\n\n| Name | Value | Category | Origin | Named by | Uses | Spellings merged |\n| --- | --- | --- | --- | --- | ---: | ---: |\n");
    for t in &r.tokens {
        out.push_str(&format!(
            "| `{}` | `{}` | {} | {} | {} | {} | {}{} |\n",
            t.name,
            t.value.replace('|', "\\|"),
            t.category.label(),
            format!("{:?}", t.origin).to_lowercase(),
            t.basis.label(),
            t.uses,
            t.merged_spellings.len(),
            if t.merged_across_units {
                " (px+rem)"
            } else {
                ""
            }
        ));
    }
    out.push_str("\n## Eligible but unnamed (kept literal)\n\n");
    if r.unnamed.is_empty() {
        out.push_str("None.\n");
    } else {
        out.push_str("| Category | Value | Uses | Why no token |\n| --- | --- | ---: | --- |\n");
        for u in &r.unnamed {
            out.push_str(&format!(
                "| {} | `{}` | {} | {} |\n",
                u.category.label(),
                u.value.replace('|', "\\|"),
                u.uses,
                u.reason
            ));
        }
    }
    out.push_str("\n## Single-use literals (not tokenized)\n\n");
    for (c, n) in &s.single_use_literals {
        out.push_str(&format!("- {}: {n} distinct values\n", c.label()));
    }
    out.push_str("\n## Site variables left in place\n\n| Variable | Reason |\n| --- | --- |\n");
    let mut by_reason: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for k in &r.kept_vars {
        by_reason
            .entry(k.reason.as_str())
            .or_default()
            .push(k.name.as_str());
    }
    for (reason, names) in &by_reason {
        let shown: Vec<&str> = names.iter().take(6).copied().collect();
        out.push_str(&format!(
            "| {} variable(s): `{}`{} | {} |\n",
            names.len(),
            shown.join("`, `"),
            if names.len() > shown.len() {
                ", …"
            } else {
                ""
            },
            reason
        ));
    }
    out.push_str("\n## Resolver check (cascade vs captured computed style)\n\n| Snapshot | Compared | Matched | Mismatched | Undecided (uncertain) | Not comparable |\n| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for c in &r.resolver_validation {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            c.snapshot,
            c.compared,
            c.matched,
            c.mismatched,
            c.skipped_uncertain,
            c.skipped_not_comparable + c.skipped_opaque
        ));
    }
    out.push_str(&format!(
        "\n## Rewrite equality (original vs tokenized cascade)\n\n{} of {} comparisons involve a rewritten declaration or a token definition; the rest are untouched declarations. {} derived tokens are referenced by at least one compared element, {} are not (listed under Unexercised tokens).\n\n",
        r.summary.rewritten_comparisons, r.summary.total_comparisons, r.summary.derived_tokens_exercised, r.summary.derived_tokens_unexercised
    ));
    out.push_str("| Snapshot | States | Compared | Equal | Different | Introduced by tokens | Undecided |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for c in &r.rewrite_equality {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            c.snapshot,
            c.states,
            c.compared,
            c.equal,
            c.different,
            c.introduced_by_tokens,
            c.skipped_uncertain
        ));
    }
    out.push_str("\n## Unexercised tokens\n\nDerived tokens that no compared element references (their replacements rest on the value merge alone, not on an equality comparison):\n\n");
    if r.unexercised_tokens.is_empty() {
        out.push_str("None.\n");
    } else {
        for name in &r.unexercised_tokens {
            out.push_str(&format!("- `{name}`\n"));
        }
    }
    out.push_str("\n## Explicit gaps\n\n");
    for g in &r.gaps {
        out.push_str(&format!("- {g}\n"));
    }
    out.push_str("\n## Environment assumptions used to evaluate conditions\n\n");
    for a in &r.media_assumptions {
        out.push_str(&format!("- {a}\n"));
    }
    out
}

/// Write an output tree to a directory that must not exist yet.
pub fn write_output(out: &Output, dir: &Path) -> Result<()> {
    if dir.exists() {
        bail!("{} already exists; choose a new directory", dir.display());
    }
    for (relative, content) in &out.files {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().context("no parent")?)?;
        std::fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_drop_bundler_hashes_and_never_repeat() {
        let mut taken = BTreeSet::new();
        assert_eq!(
            sheet_name(
                Some("https://x.test/assets/styles-B8ae2haK.css"),
                &mut taken
            ),
            "styles"
        );
        assert_eq!(
            sheet_name(
                Some("https://y.test/assets/styles-B8ae2haK.css"),
                &mut taken
            ),
            "y-test-styles"
        );
        assert_eq!(
            sheet_name(Some("https://x.test/a/careers-D2T7IprR.css"), &mut taken),
            "careers"
        );
        assert_eq!(sheet_name(None, &mut taken), "inline");
        assert_eq!(sheet_name(None, &mut taken), "inline-2");
        assert_eq!(
            sheet_name(Some("https://x.test/blog-post.css"), &mut taken),
            "blog-post"
        );
        assert_eq!(
            sheet_name(
                Some("https://x.test/p/product-features-BWKZ-Z7S.css"),
                &mut taken
            ),
            "product-features"
        );
        assert_eq!(
            sheet_name(Some("https://x.test/p/tabs-CfKLwSZY.css"), &mut taken),
            "tabs"
        );
        assert_eq!(
            sheet_name(Some("https://x.test/p/blog-post-entry.css"), &mut taken),
            "blog-post-entry"
        );
    }
}
