//! Text utilities, spec validation helpers and output writing.
use super::*;
use anyhow::{Context, Result, bail, ensure};
use oxc_allocator::Allocator;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_SOURCE,
        "{} is not a regular file within the byte budget",
        path.display()
    );
    Ok(std::fs::read(path)?)
}
pub(super) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !RESERVED.contains(&name)
}
pub(super) fn ts_program<'a>(arena: &'a Allocator, text: &'a str) -> Option<Program<'a>> {
    let parsed = Parser::new(arena, text, SourceType::ts()).parse();
    parsed.diagnostics.is_empty().then_some(parsed.program)
}
/// A TypeScript type: it must parse as the whole right-hand side of one alias.
pub(super) fn valid_type(text: &str) -> bool {
    if text.trim().is_empty()
        || text.contains(['\n', ';', '`', '\r'])
        || text.contains("//")
        || text.contains("/*")
    {
        return false;
    }
    let source = format!("type __T = {text};");
    let arena = Allocator::default();
    ts_program(&arena, &source).is_some_and(|program| {
        matches!(
            program.body.as_slice(),
            [Statement::TSTypeAliasDeclaration(_)]
        )
    })
}
/// One `import type ...;` declaration; returns the local names it binds.
pub(super) fn import_names(text: &str) -> Option<Vec<String>> {
    if text.contains(['\n', '\r']) || !text.starts_with("import type ") || !text.ends_with(';') {
        return None;
    }
    let arena = Allocator::default();
    let program = ts_program(&arena, text)?;
    let [Statement::ImportDeclaration(import)] = program.body.as_slice() else {
        return None;
    };
    (import.import_kind == ImportOrExportKind::Type).then(|| {
        import
            .specifiers
            .iter()
            .flatten()
            .map(|s| s.local().name.to_string())
            .collect()
    })
}
/// One `export type Name = ...;` declaration; returns `Name`.
pub(super) fn declaration_name(text: &str) -> Option<String> {
    if text.contains(['\n', '\r', '`']) || !text.starts_with("export type ") || !text.ends_with(';')
    {
        return None;
    }
    let arena = Allocator::default();
    let program = ts_program(&arena, text)?;
    let [Statement::ExportDeclaration(export)] = program.body.as_slice() else {
        return None;
    };
    match &export.declaration {
        Declaration::TSTypeAliasDeclaration(alias) => Some(alias.id.name.to_string()),
        _ => None,
    }
}
pub(super) fn valid_doc(text: &str) -> bool {
    !text.contains("*/")
}
/// Marks a line break inside a string or template literal, which must not be re-indented.
pub(super) const KEEP_BREAK: char = '\u{E000}';
/// Rejects TypeScript-only parses of JavaScript bodies (`a<b>(c)` is a generic call in `.ts`).
pub(super) struct TsOnly(pub(super) Vec<&'static str>);
impl<'a> Visit<'a> for TsOnly {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if it.type_arguments.is_some() {
            self.0.push("a generic call");
        }
        walk::walk_call_expression(self, it);
    }
    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if it.type_arguments.is_some() {
            self.0.push("a generic `new`");
        }
        walk::walk_new_expression(self, it);
    }
    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        if it.type_arguments.is_some() {
            self.0.push("a generic tagged template");
        }
        walk::walk_tagged_template_expression(self, it);
    }
    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        self.0.push("an `as` expression");
        walk::walk_ts_as_expression(self, it);
    }
    fn visit_ts_satisfies_expression(&mut self, it: &TSSatisfiesExpression<'a>) {
        self.0.push("a `satisfies` expression");
        walk::walk_ts_satisfies_expression(self, it);
    }
    fn visit_ts_type_assertion(&mut self, it: &TSTypeAssertion<'a>) {
        self.0.push("a type assertion");
        walk::walk_ts_type_assertion(self, it);
    }
    fn visit_ts_non_null_expression(&mut self, it: &TSNonNullExpression<'a>) {
        self.0.push("a non-null assertion");
        walk::walk_ts_non_null_expression(self, it);
    }
    fn visit_ts_instantiation_expression(&mut self, it: &TSInstantiationExpression<'a>) {
        self.0.push("an instantiation expression");
        walk::walk_ts_instantiation_expression(self, it);
    }
}

/// Counts leading whitespace characters.
pub(super) fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == '\t' || *c == ' ').count()
}
pub(super) fn convert(line: &str, remove: usize) -> String {
    let rest = &line[remove.min(line.len())..];
    let width = indent(rest);
    let tabs = rest[..width].chars().filter(|c| *c == '\t').count();
    format!(
        "{}{}",
        "  ".repeat(tabs) + &" ".repeat(width - tabs),
        &rest[width..]
    )
}
pub(super) fn reindent_block(inner: &str) -> String {
    let lines: Vec<&str> = inner.lines().collect();
    let min = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("  {}", convert(l, min))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}
pub(super) fn reindent_expression(expression: &str) -> String {
    let mut lines = expression.trim().lines();
    let first = lines.next().unwrap_or("").trim();
    let rest: Vec<&str> = lines.collect();
    let min = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    let mut out = format!("  return {first}");
    for line in rest {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(&format!("  {}", convert(line, min)));
        }
    }
    out.push(';');
    out
}

pub(super) fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map_or(0, |i| i + 1)
}
/// Applies the edits that fall inside `start..end`; an edit that straddles a
/// boundary is an error.
pub(super) fn apply(
    source: &str,
    start: usize,
    end: usize,
    edits: &[&Edit],
    literals: &[Span],
    what: &str,
) -> Result<String> {
    let mut inside: Vec<&&Edit> = Vec::new();
    for e in edits {
        if e.start >= start && e.end <= end {
            inside.push(e);
        } else if e.start < end && e.end > start {
            bail!("an edit straddles a boundary of the extracted code in `{what}`");
        }
    }
    inside.sort_by_key(|e| (e.start, e.end));
    let copy = |text: &mut String, from: usize, to: usize| {
        for (offset, c) in source[from..to].char_indices() {
            let at = from + offset;
            let in_literal = c == '\n'
                && literals
                    .iter()
                    .any(|l| (l.start as usize) < at && at < l.end as usize);
            text.push(if in_literal { KEEP_BREAK } else { c });
        }
    };
    let mut text = String::new();
    let mut cursor = start;
    for e in inside {
        ensure!(e.start >= cursor, "overlapping edits in `{what}`");
        copy(&mut text, cursor, e.start);
        text.push_str(&e.text);
        cursor = e.end;
    }
    copy(&mut text, cursor, end);
    Ok(text)
}

/// Writes both outputs or neither: each goes to a temporary sibling first, then is
/// linked into place (which fails if the destination exists). Never leaves a lone module.
pub(super) fn write_pair(
    first: &Path,
    first_bytes: &[u8],
    second: &Path,
    second_bytes: &[u8],
) -> Result<()> {
    ensure!(
        first != second,
        "the module and manifest paths are the same"
    );
    for path in [first, second] {
        ensure!(
            std::fs::symlink_metadata(path).is_err(),
            "create {} (it must not already exist)",
            path.display()
        );
    }
    let stage = |path: &Path, bytes: &[u8]| -> Result<PathBuf> {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".tmp{}", std::process::id()));
        let tmp = path.with_file_name(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes)?;
        Ok(tmp)
    };
    let mut temporaries = Vec::new();
    let mut linked: Vec<&Path> = Vec::new();
    let result = (|| -> Result<()> {
        for (path, bytes) in [(first, first_bytes), (second, second_bytes)] {
            temporaries.push(stage(path, bytes)?);
        }
        for (path, tmp) in [first, second].into_iter().zip(&temporaries) {
            std::fs::hard_link(tmp, path).with_context(|| {
                format!("create {} (it must not already exist)", path.display())
            })?;
            linked.push(path);
        }
        Ok(())
    })();
    for tmp in &temporaries {
        let _ = std::fs::remove_file(tmp);
    }
    if result.is_err() {
        for path in linked {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}
