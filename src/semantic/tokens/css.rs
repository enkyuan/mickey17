//! CSS syntax tree built on `cssparser`'s tokenizer (no regex or string patching).
//!
//! The tree keeps every declaration value as a tree of component values so a
//! stylesheet can be rewritten structurally and serialized again. Rules the
//! parser cannot interpret are kept as [`Node::Raw`] with the reason; nothing is
//! silently dropped.

use super::selector::{self, SelectorList};
use cssparser::{Delimiter, ParseError, Parser, ToCss, Token};

/// An owned CSS token (cssparser's tokens borrow reference-counted input).
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    AtKeyword(String),
    Hash(String),
    IdHash(String),
    Str(String),
    Url(String),
    Delim(char),
    Number {
        value: f32,
        int_value: Option<i32>,
        has_sign: bool,
    },
    Percentage {
        unit_value: f32,
        int_value: Option<i32>,
        has_sign: bool,
    },
    Dimension {
        value: f32,
        int_value: Option<i32>,
        has_sign: bool,
        unit: String,
    },
    Space,
    Colon,
    Semicolon,
    Comma,
    IncludeMatch,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
    Cdo,
    Cdc,
    /// Bad strings/urls and stray closing brackets, kept as serialized text.
    Other(String),
}

impl Tok {
    fn from_token(token: &Token<'_>) -> Tok {
        match token {
            Token::Ident(v) => Tok::Ident(v.to_string()),
            Token::AtKeyword(v) => Tok::AtKeyword(v.to_string()),
            Token::Hash(v) => Tok::Hash(v.to_string()),
            Token::IDHash(v) => Tok::IdHash(v.to_string()),
            Token::QuotedString(v) => Tok::Str(v.to_string()),
            Token::UnquotedUrl(v) => Tok::Url(v.to_string()),
            Token::Delim(c) => Tok::Delim(*c),
            Token::Number {
                value,
                int_value,
                has_sign,
            } => Tok::Number {
                value: *value,
                int_value: *int_value,
                has_sign: *has_sign,
            },
            Token::Percentage {
                unit_value,
                int_value,
                has_sign,
            } => Tok::Percentage {
                unit_value: *unit_value,
                int_value: *int_value,
                has_sign: *has_sign,
            },
            Token::Dimension {
                value,
                int_value,
                has_sign,
                unit,
            } => Tok::Dimension {
                value: *value,
                int_value: *int_value,
                has_sign: *has_sign,
                unit: unit.to_string(),
            },
            Token::WhiteSpace(_) => Tok::Space,
            Token::Colon => Tok::Colon,
            Token::Semicolon => Tok::Semicolon,
            Token::Comma => Tok::Comma,
            Token::IncludeMatch => Tok::IncludeMatch,
            Token::DashMatch => Tok::DashMatch,
            Token::PrefixMatch => Tok::PrefixMatch,
            Token::SuffixMatch => Tok::SuffixMatch,
            Token::SubstringMatch => Tok::SubstringMatch,
            Token::CDO => Tok::Cdo,
            Token::CDC => Tok::Cdc,
            other => Tok::Other(other.to_css_string()),
        }
    }

    pub fn to_css(&self) -> String {
        let token = match self {
            Tok::Ident(v) => Token::Ident(v.clone().into()),
            Tok::AtKeyword(v) => Token::AtKeyword(v.clone().into()),
            Tok::Hash(v) => Token::Hash(v.clone().into()),
            Tok::IdHash(v) => Token::IDHash(v.clone().into()),
            Tok::Str(v) => Token::QuotedString(v.clone().into()),
            Tok::Url(v) => Token::UnquotedUrl(v.clone().into()),
            Tok::Delim(c) => Token::Delim(*c),
            Tok::Number {
                value,
                int_value,
                has_sign,
            } => Token::Number {
                value: *value,
                int_value: *int_value,
                has_sign: *has_sign,
            },
            Tok::Percentage {
                unit_value,
                int_value,
                has_sign,
            } => Token::Percentage {
                unit_value: *unit_value,
                int_value: *int_value,
                has_sign: *has_sign,
            },
            Tok::Dimension {
                value,
                int_value,
                has_sign,
                unit,
            } => Token::Dimension {
                value: *value,
                int_value: *int_value,
                has_sign: *has_sign,
                unit: unit.clone().into(),
            },
            Tok::Space => return " ".into(),
            Tok::Colon => Token::Colon,
            Tok::Semicolon => Token::Semicolon,
            Tok::Comma => Token::Comma,
            Tok::IncludeMatch => Token::IncludeMatch,
            Tok::DashMatch => Token::DashMatch,
            Tok::PrefixMatch => Token::PrefixMatch,
            Tok::SuffixMatch => Token::SuffixMatch,
            Tok::SubstringMatch => Token::SubstringMatch,
            Tok::Cdo => Token::CDO,
            Tok::Cdc => Token::CDC,
            Tok::Other(text) => return text.clone(),
        };
        token.to_css_string()
    }
}

/// One component value (CSS Syntax Level 3): a token or a simple block/function.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    Tok(Tok),
    Func(String, Vec<Component>),
    Paren(Vec<Component>),
    Square(Vec<Component>),
    Curly(Vec<Component>),
}

impl Component {
    pub fn ident(&self) -> Option<&str> {
        match self {
            Component::Tok(Tok::Ident(name)) => Some(name),
            _ => None,
        }
    }
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Component::Tok(Tok::Space))
    }
    pub fn is_comma(&self) -> bool {
        matches!(self, Component::Tok(Tok::Comma))
    }
    pub fn write(&self, out: &mut String) {
        match self {
            Component::Tok(token) => out.push_str(&token.to_css()),
            Component::Func(name, inner) => {
                out.push_str(&Token::Function(name.clone().into()).to_css_string());
                write_all(inner, out);
                out.push(')');
            }
            Component::Paren(inner) => {
                out.push('(');
                write_all(inner, out);
                out.push(')');
            }
            Component::Square(inner) => {
                out.push('[');
                write_all(inner, out);
                out.push(']');
            }
            Component::Curly(inner) => {
                out.push('{');
                write_all(inner, out);
                out.push('}');
            }
        }
    }
}

pub fn write_all(components: &[Component], out: &mut String) {
    for component in components {
        component.write(out);
    }
}

pub fn to_css(components: &[Component]) -> String {
    let mut out = String::new();
    write_all(components, &mut out);
    out
}

/// Remove leading/trailing whitespace and collapse each whitespace run to one token.
pub fn trim(components: &[Component]) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    for component in components {
        if component.is_whitespace() {
            if out.is_empty() || out.last().is_some_and(Component::is_whitespace) {
                continue;
            }
            out.push(Component::Tok(Tok::Space));
        } else {
            out.push(component.clone());
        }
    }
    while out.last().is_some_and(Component::is_whitespace) {
        out.pop();
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// Lower-cased for standard properties, exact for `--custom` properties.
    pub name: String,
    pub value: Vec<Component>,
    pub important: bool,
    /// The value exactly as written in the source (without `!important`). Output uses it
    /// verbatim while the declaration is unmodified, so serialization quirks of the token
    /// tree (numbers, `U+` ranges, glued tokens) can never alter untouched values. A
    /// rewritten declaration carries `None`.
    pub raw: Option<String>,
}

impl Declaration {
    pub fn is_custom(&self) -> bool {
        self.name.starts_with("--")
    }
    pub fn value_css(&self) -> String {
        to_css(&self.value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Decl(Declaration),
    Rule(Node),
}

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: SelectorList,
    pub items: Vec<Item>,
    /// Selector text as written; output uses it verbatim (type and attribute name case,
    /// attribute flags and escapes survive exactly).
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    /// Lower-case name without `@`.
    pub name: String,
    pub prelude: Vec<Component>,
    /// The prelude exactly as written; used for output while present.
    pub raw_prelude: Option<String>,
    /// `None` for statement at-rules (`@import`, `@layer a, b;`).
    pub items: Option<Vec<Item>>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Style(StyleRule),
    At(AtRule),
    /// A keyframe block inside `@keyframes` (`from`, `50%`, `0%, 100%`).
    Frame {
        selector: String,
        items: Vec<Item>,
    },
    /// Text the parser could not interpret; kept for diagnostics and verbatim output.
    Raw {
        text: String,
        reason: String,
    },
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Node::Style(a), Node::Style(b)) => {
                a.selectors.to_css() == b.selectors.to_css() && a.items == b.items
            }
            (Node::At(a), Node::At(b)) => a == b,
            (
                Node::Frame {
                    selector: a,
                    items: x,
                },
                Node::Frame {
                    selector: b,
                    items: y,
                },
            ) => a == b && x == y,
            (Node::Raw { text: a, .. }, Node::Raw { text: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub nodes: Vec<Node>,
}

type Err = ParseError<()>;

fn components<'i>(parser: &mut Parser<'i>) -> Result<Vec<Component>, Err> {
    let mut out = Vec::new();
    while let Ok(token) = parser.next_including_whitespace() {
        let token = token.clone();
        match token {
            Token::Function(name) => {
                let inner = parser.parse_nested_block(components)?;
                out.push(Component::Func(name.to_string(), inner));
            }
            Token::ParenthesisBlock => {
                out.push(Component::Paren(parser.parse_nested_block(components)?));
            }
            Token::SquareBracketBlock => {
                out.push(Component::Square(parser.parse_nested_block(components)?));
            }
            Token::CurlyBracketBlock => {
                out.push(Component::Curly(parser.parse_nested_block(components)?));
            }
            other => out.push(Component::Tok(Tok::from_token(&other))),
        }
    }
    Ok(out)
}

pub(super) fn parse_components_in<'i>(parser: &mut Parser<'i>) -> Result<Vec<Component>, Err> {
    components(parser)
}

/// Parse a component-value list (a declaration value, an at-rule prelude, ...).
pub fn parse_components(text: &str) -> Vec<Component> {
    let mut parser = Parser::new(text);
    components(&mut parser).unwrap_or_default()
}

fn split_important(mut value: Vec<Component>) -> (Vec<Component>, bool) {
    let mut end = value.len();
    while end > 0 && value[end - 1].is_whitespace() {
        end -= 1;
    }
    if end >= 2
        && value[end - 1]
            .ident()
            .is_some_and(|i| i.eq_ignore_ascii_case("important"))
    {
        let mut bang = end - 1;
        while bang > 0 && value[bang - 1].is_whitespace() {
            bang -= 1;
        }
        if bang > 0 && matches!(value[bang - 1], Component::Tok(Tok::Delim('!'))) {
            value.truncate(bang - 1);
            return (trim(&value), true);
        }
    }
    (trim(&value), false)
}

/// The text after the first token of `text` (an at-keyword or a property name).
fn after_first_token(text: &str) -> &str {
    let mut parser = Parser::new(text);
    if parser.next_including_whitespace().is_err() {
        return "";
    }
    &text[parser.position().byte_index()..]
}

/// The value of `name: value [!important]` as written: after the colon, before a trailing
/// `!important`, located by token position (so `!` inside strings, comments or escaped names
/// can never confuse it).
fn raw_value(source: &str) -> Option<String> {
    let mut parser = Parser::new(source);
    match parser.next_including_whitespace() {
        Ok(Token::Ident(_)) => {}
        _ => return None,
    }
    parser.skip_whitespace();
    match parser.next_including_whitespace() {
        Ok(Token::Colon) => {}
        _ => return None,
    }
    let start = parser.position().byte_index();
    // (start byte, is `!`, is ident `important`) of every significant token.
    let mut tokens: Vec<(usize, bool, bool)> = Vec::new();
    loop {
        let at = parser.position().byte_index();
        match parser.next_including_whitespace() {
            Ok(Token::WhiteSpace(_)) => {}
            Ok(Token::Delim('!')) => tokens.push((at, true, false)),
            Ok(Token::Ident(i)) => tokens.push((at, false, i.eq_ignore_ascii_case("important"))),
            Ok(_) => tokens.push((at, false, false)),
            Err(_) => break,
        }
    }
    let mut end = source.len();
    if let [.., (bang, true, false), (_, false, true)] = tokens.as_slice() {
        end = *bang;
    }
    Some(source[start..end.max(start)].trim().to_string())
}

fn declaration(prelude: &[Component], source: &str) -> Option<Declaration> {
    let mut iter = prelude.iter().skip_while(|c| c.is_whitespace());
    let name = iter.next()?.ident()?.to_string();
    let rest: Vec<&Component> = iter.collect();
    let mut rest = rest.into_iter().skip_while(|c| c.is_whitespace());
    match rest.next()? {
        Component::Tok(Tok::Colon) => {}
        _ => return None,
    }
    let value: Vec<Component> = rest.cloned().collect();
    let (value, important) = split_important(value);
    let name = if name.starts_with("--") {
        name
    } else {
        name.to_ascii_lowercase()
    };
    let raw = raw_value(source);
    Some(Declaration {
        name,
        value,
        important,
        raw,
    })
}

fn parse_items<'i>(parser: &mut Parser<'i>, top_level: bool, keyframes: bool) -> Vec<Item> {
    let mut items = Vec::new();
    loop {
        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }
        let state = parser.state();
        if matches!(parser.next_including_whitespace(), Ok(Token::Semicolon)) {
            continue;
        }
        parser.reset(&state);
        let start = parser.position();
        let prelude = parser.parse_until_before(
            Delimiter::CurlyBracketBlock | Delimiter::Semicolon,
            components,
        );
        let text = parser.slice_from(start).trim().to_string();
        let prelude = match prelude {
            Ok(prelude) => prelude,
            Err(_) => {
                // Consume the offending block/statement so parsing can continue.
                let _ = parser.next_including_whitespace();
                items.push(Item::Rule(Node::Raw {
                    text,
                    reason: "unparsable prelude".into(),
                }));
                continue;
            }
        };
        let first = prelude.iter().find(|c| !c.is_whitespace());
        let at_name = match first {
            Some(Component::Tok(Tok::AtKeyword(name))) => Some(name.to_ascii_lowercase()),
            _ => None,
        };
        let inner_keyframes = at_name.as_deref().is_some_and(|n| n.ends_with("keyframes"));
        let mut body: Option<Vec<Item>> = None;
        let mut body_failed = false;
        if let Ok(Token::CurlyBracketBlock) = parser.next_including_whitespace() {
            match parser
                .parse_nested_block(|p| Ok::<_, Err>(parse_items(p, false, inner_keyframes)))
            {
                Ok(inner) => body = Some(inner),
                Err(_) => body_failed = true,
            }
        }
        // Raw nodes keep the whole source (prelude and block), never only the prelude.
        let full = if body.is_some() || body_failed {
            parser.slice_from(start).trim().to_string()
        } else {
            text.clone()
        };
        if body_failed {
            items.push(Item::Rule(Node::Raw {
                text: full,
                reason: "unparsable block".into(),
            }));
            continue;
        }
        if let Some(name) = at_name {
            let skip = prelude.iter().position(|c| !c.is_whitespace()).unwrap_or(0) + 1;
            items.push(Item::Rule(Node::At(AtRule {
                name,
                prelude: trim(&prelude[skip..]),
                raw_prelude: Some(after_first_token(&text).trim().to_string()),
                items: body,
            })));
            continue;
        }
        match body {
            None => match declaration(&prelude, &text) {
                Some(decl) if !top_level => items.push(Item::Decl(decl)),
                Some(_) => items.push(Item::Rule(Node::Raw {
                    text,
                    reason: "declaration outside a rule".into(),
                })),
                None => items.push(Item::Rule(Node::Raw {
                    text,
                    reason: "not a declaration or rule".into(),
                })),
            },
            Some(inner) if keyframes => items.push(Item::Rule(Node::Frame {
                selector: text.clone(),
                items: inner,
            })),
            Some(inner) => {
                if !top_level
                    && matches!(prelude.iter().find(|c| !c.is_whitespace()), Some(c) if c.ident().is_some_and(|i| i.starts_with("--")))
                    && declaration(&prelude, &text).is_some()
                {
                    items.push(Item::Rule(Node::Raw {
                        text: full,
                        reason: "custom property with a {} block value".into(),
                    }));
                    continue;
                }
                match selector::parse_list(&text) {
                    Ok(selectors) => items.push(Item::Rule(Node::Style(StyleRule {
                        selectors,
                        items: inner,
                        source: text.clone(),
                    }))),
                    Err(reason) => items.push(Item::Rule(Node::Raw { text: full, reason })),
                }
            }
        }
    }
    items
}

/// Parse one stylesheet text (any number of top-level rules).
pub fn parse_sheet(text: &str) -> Sheet {
    let mut parser = Parser::new(text);
    let nodes = parse_items(&mut parser, true, false)
        .into_iter()
        .filter_map(|item| match item {
            Item::Rule(node) => Some(node),
            Item::Decl(_) => None,
        })
        .collect();
    Sheet { nodes }
}

/// Parse an inline `style=""` attribute value.
pub fn parse_declarations(text: &str) -> Vec<Declaration> {
    let mut parser = Parser::new(text);
    parse_items(&mut parser, false, false)
        .into_iter()
        .filter_map(|item| match item {
            Item::Decl(decl) => Some(decl),
            Item::Rule(_) => None,
        })
        .collect()
}

// ----- serialization -------------------------------------------------------

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_items(items: &[Item], depth: usize, out: &mut String) {
    for item in items {
        match item {
            Item::Decl(decl) => {
                indent(out, depth);
                out.push_str(&Token::Ident(decl.name.clone().into()).to_css_string());
                out.push_str(": ");
                match &decl.raw {
                    Some(raw) => out.push_str(raw),
                    None => write_all(&decl.value, out),
                }
                if decl.important {
                    out.push_str(" !important");
                }
                out.push_str(";\n");
            }
            Item::Rule(node) => write_node(node, depth, out),
        }
    }
}

pub fn write_node(node: &Node, depth: usize, out: &mut String) {
    match node {
        Node::Raw { text, .. } => {
            indent(out, depth);
            out.push_str(text);
            // Inside a rule body a raw declaration (`*zoom:1`, `--x:{a:b}`) needs its `;` or it
            // would swallow the next declaration.
            out.push_str(if depth > 0 { ";\n" } else { "\n" });
        }
        Node::Frame { selector, items } => {
            indent(out, depth);
            out.push_str(selector);
            out.push_str(" {\n");
            write_items(items, depth + 1, out);
            indent(out, depth);
            out.push_str("}\n");
        }
        Node::Style(rule) => {
            indent(out, depth);
            if rule.source.is_empty() {
                out.push_str(&rule.selectors.to_css());
            } else {
                out.push_str(&rule.source);
            }
            out.push_str(" {\n");
            write_items(&rule.items, depth + 1, out);
            indent(out, depth);
            out.push_str("}\n");
        }
        Node::At(at) => {
            indent(out, depth);
            out.push('@');
            out.push_str(&at.name);
            match &at.raw_prelude {
                Some(raw) if !raw.is_empty() => {
                    out.push(' ');
                    out.push_str(raw);
                }
                Some(_) => {}
                None if !at.prelude.is_empty() => {
                    out.push(' ');
                    write_all(&at.prelude, out);
                }
                None => {}
            }
            match &at.items {
                None => out.push_str(";\n"),
                Some(items) => {
                    out.push_str(" {\n");
                    write_items(items, depth + 1, out);
                    indent(out, depth);
                    out.push_str("}\n");
                }
            }
        }
    }
}

/// Number of unparsable nodes anywhere in the tree.
pub fn raw_count(nodes: &[Node]) -> usize {
    fn items(items: &[Item]) -> usize {
        items
            .iter()
            .map(|i| match i {
                Item::Decl(_) => 0,
                Item::Rule(n) => raw_count(std::slice::from_ref(n)),
            })
            .sum()
    }
    nodes
        .iter()
        .map(|n| match n {
            Node::Raw { .. } => 1,
            Node::Style(r) => items(&r.items),
            Node::At(a) => a.items.as_deref().map_or(0, items),
            Node::Frame { items: i, .. } => items(i),
        })
        .sum()
}

impl Sheet {
    pub fn to_css(&self) -> String {
        let mut out = String::new();
        for node in &self.nodes {
            write_node(node, 0, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_declarations_and_important() {
        let sheet = parse_sheet(".a, .b > c { color: #fff !important; margin: 0 auto }");
        assert_eq!(sheet.nodes.len(), 1);
        let Node::Style(rule) = &sheet.nodes[0] else {
            panic!()
        };
        let Item::Decl(first) = &rule.items[0] else {
            panic!()
        };
        assert_eq!(first.name, "color");
        assert!(first.important);
        assert_eq!(first.value_css(), "#fff");
        assert_eq!(rule.selectors.to_css(), ".a, .b > c");
    }

    #[test]
    fn keeps_at_rules_and_nesting() {
        let text = "@layer base, theme;\n@media (width >= 40rem) { .a { color: red; &:hover { color: blue } .b { top: 0 } } }";
        let sheet = parse_sheet(text);
        assert_eq!(sheet.nodes.len(), 2);
        let Node::At(layer) = &sheet.nodes[0] else {
            panic!()
        };
        assert_eq!(layer.name, "layer");
        assert!(layer.items.is_none());
        let Node::At(media) = &sheet.nodes[1] else {
            panic!()
        };
        let items = media.items.as_ref().unwrap();
        let Item::Rule(Node::Style(rule)) = &items[0] else {
            panic!()
        };
        assert_eq!(rule.items.len(), 3);
        let again = parse_sheet(&sheet.to_css());
        assert_eq!(again.to_css(), sheet.to_css());
    }

    #[test]
    fn unparsable_content_is_kept_not_dropped() {
        let sheet = parse_sheet("a:unknown-pseudo-thing-(( { color: red } .ok { top: 1px }");
        assert!(
            sheet
                .nodes
                .iter()
                .any(|n| matches!(n, Node::Raw { .. }) || matches!(n, Node::Style(_)))
        );
        assert!(sheet.to_css().contains(".ok"));
    }

    #[test]
    fn source_text_survives_for_values_preludes_and_escaped_names() {
        let src = "@media (max-width: 1023.999px) { .a { --a\\:b: 1px; color: red !important /* keep! */; margin: 1.6180339887px } } @keyframes k { 33.333333% { top: 0 } }";
        let out = parse_sheet(src).to_css();
        assert!(out.contains("(max-width: 1023.999px)"), "{out}");
        assert!(out.contains("--a\\:b: 1px"), "{out}");
        assert!(out.contains("color: red !important;"), "{out}");
        assert!(out.contains("margin: 1.6180339887px"), "{out}");
        assert!(out.contains("33.333333% {"), "{out}");
        assert_eq!(raw_count(&parse_sheet(&out).nodes), 0);
    }

    #[test]
    fn custom_properties_keep_case_and_value_structure() {
        let decls =
            parse_declarations("--Brand-Ink: oklch(94.3% .016 250); COLOR: var(--Brand-Ink, #000)");
        assert_eq!(decls[0].name, "--Brand-Ink");
        assert_eq!(decls[1].name, "color");
        assert!(matches!(&decls[1].value[0], Component::Func(name, _) if name == "var"));
    }
}
