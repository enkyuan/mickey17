//! Selector AST, specificity and a three-valued matcher.
//!
//! Matching is `Yes` / `No` / `Unknown`. `Unknown` means the selector depends on
//! something a captured snapshot cannot decide (shadow hosts, language, form
//! validity, ...). Callers must treat it as a gap and never as a match or a miss.

use cssparser::{Parser, ToCss, Token, parse_nth};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

impl Combinator {
    fn css(self) -> &'static str {
        match self {
            Combinator::Descendant => " ",
            Combinator::Child => " > ",
            Combinator::NextSibling => " + ",
            Combinator::LaterSibling => " ~ ",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NthKind {
    Child,
    LastChild,
    OfType,
    LastOfType,
}

#[derive(Debug, Clone)]
pub enum Simple {
    Type(String),
    Universal,
    Class(String),
    Id(String),
    Attr {
        name: String,
        op: AttrOp,
        value: String,
        insensitive: bool,
    },
    /// A pseudo-class with no arguments, lower-case name without the colon.
    Pseudo(String),
    Is(SelectorList),
    Where(SelectorList),
    Not(SelectorList),
    Has(SelectorList),
    Nth {
        kind: NthKind,
        a: i32,
        b: i32,
        of: Option<SelectorList>,
    },
    /// A functional pseudo-class we do not evaluate (`:lang()`, `:dir()`, `:host()`).
    OtherFunction {
        name: String,
        args: String,
    },
    PseudoElement {
        name: String,
        args: Option<String>,
    },
    Nesting,
}

#[derive(Debug, Clone, Default)]
pub struct Compound {
    pub simples: Vec<Simple>,
}

impl Compound {
    fn is_empty(&self) -> bool {
        self.simples.is_empty()
    }
    pub fn pseudo_element(&self) -> Option<&str> {
        self.simples.iter().find_map(|s| match s {
            Simple::PseudoElement { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Complex {
    /// Combinator before the first compound (relative selectors only).
    pub leading: Option<Combinator>,
    pub compounds: Vec<Compound>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
}

#[derive(Debug, Clone, Default)]
pub struct SelectorList(pub Vec<Complex>);

pub type Specificity = (u32, u32, u32);

fn add(a: Specificity, b: Specificity) -> Specificity {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

impl SelectorList {
    pub fn max_specificity(&self) -> Specificity {
        self.0
            .iter()
            .map(Complex::specificity)
            .max()
            .unwrap_or((0, 0, 0))
    }
    pub fn to_css(&self) -> String {
        let mut out = String::new();
        write_list(self, &mut out);
        out
    }
}

impl Complex {
    pub fn specificity(&self) -> Specificity {
        let mut total = (0, 0, 0);
        for compound in &self.compounds {
            for simple in &compound.simples {
                total = add(total, simple.specificity());
            }
        }
        total
    }

    /// The pseudo-element the selector targets, if any.
    pub fn pseudo_element(&self) -> Option<&str> {
        self.compounds.last().and_then(Compound::pseudo_element)
    }

    pub fn to_css(&self) -> String {
        let mut out = String::new();
        write_complex(self, &mut out);
        out
    }
}

impl Simple {
    pub fn specificity(&self) -> Specificity {
        match self {
            Simple::Type(_) => (0, 0, 1),
            Simple::Universal | Simple::Nesting => (0, 0, 0),
            Simple::Id(_) => (1, 0, 0),
            Simple::Class(_) | Simple::Attr { .. } | Simple::Pseudo(_) => (0, 1, 0),
            Simple::Is(list) | Simple::Not(list) | Simple::Has(list) => list.max_specificity(),
            Simple::Where(_) => (0, 0, 0),
            Simple::Nth { of, .. } => match of {
                Some(list) => add((0, 1, 0), list.max_specificity()),
                None => (0, 1, 0),
            },
            Simple::OtherFunction { .. } => (0, 1, 0),
            Simple::PseudoElement { .. } => (0, 0, 1),
        }
    }
}

// ----- serialization -------------------------------------------------------

fn escape_ident(name: &str) -> String {
    Token::Ident(name.to_string().into()).to_css_string()
}

fn write_list(list: &SelectorList, out: &mut String) {
    for (i, complex) in list.0.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_complex(complex, out);
    }
}

fn write_complex(complex: &Complex, out: &mut String) {
    if let Some(lead) = complex.leading {
        out.push_str(lead.css().trim_start());
        if lead != Combinator::Descendant && !complex.compounds.is_empty() {
            out.push(' ');
        }
    }
    for (i, compound) in complex.compounds.iter().enumerate() {
        if i > 0 {
            out.push_str(complex.combinators[i - 1].css());
        }
        write_compound(compound, out);
    }
}

fn write_compound(compound: &Compound, out: &mut String) {
    for simple in &compound.simples {
        match simple {
            Simple::Type(name) => out.push_str(&escape_ident(name)),
            Simple::Universal => out.push('*'),
            Simple::Nesting => out.push('&'),
            Simple::Class(name) => {
                out.push('.');
                out.push_str(&escape_ident(name));
            }
            Simple::Id(name) => {
                out.push('#');
                out.push_str(&Token::Hash(name.clone().into()).to_css_string()[1..]);
            }
            Simple::Attr {
                name,
                op,
                value,
                insensitive,
            } => {
                out.push('[');
                out.push_str(&escape_ident(name));
                let symbol = match op {
                    AttrOp::Exists => "",
                    AttrOp::Equals => "=",
                    AttrOp::Includes => "~=",
                    AttrOp::Dash => "|=",
                    AttrOp::Prefix => "^=",
                    AttrOp::Suffix => "$=",
                    AttrOp::Substring => "*=",
                };
                out.push_str(symbol);
                if *op != AttrOp::Exists {
                    out.push_str(&Token::QuotedString(value.clone().into()).to_css_string());
                    if *insensitive {
                        out.push_str(" i");
                    }
                }
                out.push(']');
            }
            Simple::Pseudo(name) => {
                out.push(':');
                out.push_str(name);
            }
            Simple::Is(list) => write_fn("is", list, out),
            Simple::Where(list) => write_fn("where", list, out),
            Simple::Not(list) => write_fn("not", list, out),
            Simple::Has(list) => write_fn("has", list, out),
            Simple::Nth { kind, a, b, of } => {
                out.push(':');
                out.push_str(match kind {
                    NthKind::Child => "nth-child(",
                    NthKind::LastChild => "nth-last-child(",
                    NthKind::OfType => "nth-of-type(",
                    NthKind::LastOfType => "nth-last-of-type(",
                });
                out.push_str(&format!("{a}n{b:+}"));
                if let Some(list) = of {
                    out.push_str(" of ");
                    write_list(list, out);
                }
                out.push(')');
            }
            Simple::OtherFunction { name, args } => {
                out.push_str(&format!(":{name}({args})"));
            }
            Simple::PseudoElement { name, args } => match args {
                Some(args) => out.push_str(&format!("::{name}({args})")),
                None => out.push_str(&format!("::{name}")),
            },
        }
    }
}

fn write_fn(name: &str, list: &SelectorList, out: &mut String) {
    out.push(':');
    out.push_str(name);
    out.push('(');
    write_list(list, out);
    out.push(')');
}

// ----- parsing -------------------------------------------------------------

type PResult<T> = Result<T, cssparser::ParseError<()>>;

/// Parse a (possibly relative, possibly nested) selector list.
pub fn parse_list(text: &str) -> Result<SelectorList, String> {
    let mut parser = Parser::new(text);
    let list = parser
        .parse_entirely(|p| list_in(p, true))
        .map_err(|e| format!("selector `{}`: {:?}", text.trim(), e.kind))?;
    if list.0.is_empty() {
        return Err(format!("selector `{}`: empty", text.trim()));
    }
    Ok(list)
}

/// Forgiving selector list (`:is()`, `:where()`): an empty list is valid and matches nothing.
fn forgiving_in<'i>(p: &mut Parser<'i>) -> PResult<SelectorList> {
    p.skip_whitespace();
    if p.is_exhausted() {
        return Ok(SelectorList(Vec::new()));
    }
    list_in(p, false)
}

fn list_in<'i>(p: &mut Parser<'i>, relative: bool) -> PResult<SelectorList> {
    let items = p.parse_comma_separated(|p| complex_in(p, relative))?;
    Ok(SelectorList(items))
}

fn complex_in<'i>(p: &mut Parser<'i>, relative: bool) -> PResult<Complex> {
    let mut complex = Complex::default();
    let mut current = Compound::default();
    let mut link: Option<Combinator> = None;
    let mut explicit_pending = false;
    while let Ok(token) = p.next_including_whitespace() {
        let token = token.clone();
        match token {
            Token::WhiteSpace(_) => {
                if !current.is_empty() {
                    complex.compounds.push(std::mem::take(&mut current));
                    link = Some(Combinator::Descendant);
                }
            }
            Token::Delim(c @ ('>' | '+' | '~')) => {
                let combinator = match c {
                    '>' => Combinator::Child,
                    '+' => Combinator::NextSibling,
                    _ => Combinator::LaterSibling,
                };
                if !current.is_empty() {
                    complex.compounds.push(std::mem::take(&mut current));
                } else if complex.compounds.is_empty() {
                    if !relative || complex.leading.is_some() {
                        return Err(cssparser::ParseError::custom(()));
                    }
                    complex.leading = Some(combinator);
                    link = None;
                    explicit_pending = false;
                    continue;
                }
                link = Some(combinator);
                explicit_pending = true;
            }
            other => {
                if current.is_empty() && !complex.compounds.is_empty() {
                    complex.combinators.push(
                        link.take()
                            .ok_or_else(|| cssparser::ParseError::custom(()))?,
                    );
                    explicit_pending = false;
                }
                simple_in(p, other, &mut current)?;
            }
        }
    }
    if !current.is_empty() {
        complex.compounds.push(current);
    }
    if complex.compounds.is_empty() || explicit_pending {
        return Err(cssparser::ParseError::custom(()));
    }
    Ok(complex)
}

fn simple_in<'i>(p: &mut Parser<'i>, token: Token<'i>, compound: &mut Compound) -> PResult<()> {
    let simple = match token {
        Token::Ident(name) => Simple::Type(name.to_ascii_lowercase()),
        Token::Delim('*') => Simple::Universal,
        Token::Delim('&') => Simple::Nesting,
        Token::Delim('.') => match p.next_including_whitespace()?.clone() {
            Token::Ident(name) => Simple::Class(name.to_string()),
            _ => return Err(cssparser::ParseError::custom(())),
        },
        Token::IDHash(name) | Token::Hash(name) => Simple::Id(name.to_string()),
        Token::SquareBracketBlock => p.parse_nested_block(attribute)?,
        Token::Colon => pseudo(p)?,
        _ => return Err(cssparser::ParseError::custom(())),
    };
    compound.simples.push(simple);
    Ok(())
}

fn attribute<'i>(p: &mut Parser<'i>) -> PResult<Simple> {
    p.skip_whitespace();
    let name = p.expect_ident_cloned()?.to_string();
    p.skip_whitespace();
    let op = match p.next() {
        Err(_) => {
            return Ok(Simple::Attr {
                name,
                op: AttrOp::Exists,
                value: String::new(),
                insensitive: false,
            });
        }
        Ok(Token::Delim('=')) => AttrOp::Equals,
        Ok(Token::IncludeMatch) => AttrOp::Includes,
        Ok(Token::DashMatch) => AttrOp::Dash,
        Ok(Token::PrefixMatch) => AttrOp::Prefix,
        Ok(Token::SuffixMatch) => AttrOp::Suffix,
        Ok(Token::SubstringMatch) => AttrOp::Substring,
        Ok(_) => return Err(cssparser::ParseError::custom(())),
    };
    let value = p.expect_ident_or_string()?.to_string();
    p.skip_whitespace();
    let insensitive = match p.next() {
        Ok(Token::Ident(flag)) if flag.eq_ignore_ascii_case("i") => true,
        Ok(Token::Ident(flag)) if flag.eq_ignore_ascii_case("s") => false,
        Ok(_) => return Err(cssparser::ParseError::custom(())),
        Err(_) => false,
    };
    Ok(Simple::Attr {
        name: name.to_ascii_lowercase(),
        op,
        value,
        insensitive,
    })
}

fn raw_args<'i>(p: &mut Parser<'i>) -> PResult<String> {
    let components = super::css::parse_components_in(p)?;
    Ok(super::css::to_css(&super::css::trim(&components)))
}

fn pseudo<'i>(p: &mut Parser<'i>) -> PResult<Simple> {
    let token = p.next_including_whitespace()?.clone();
    match token {
        Token::Colon => match p.next_including_whitespace()?.clone() {
            Token::Ident(name) => Ok(Simple::PseudoElement {
                name: name.to_ascii_lowercase(),
                args: None,
            }),
            Token::Function(name) => {
                let args = p.parse_nested_block(raw_args)?;
                Ok(Simple::PseudoElement {
                    name: name.to_ascii_lowercase(),
                    args: Some(args),
                })
            }
            _ => Err(cssparser::ParseError::custom(())),
        },
        Token::Ident(name) => {
            let lower = name.to_ascii_lowercase();
            // Legacy single-colon pseudo-elements.
            if matches!(
                lower.as_str(),
                "before" | "after" | "first-line" | "first-letter"
            ) {
                Ok(Simple::PseudoElement {
                    name: lower,
                    args: None,
                })
            } else {
                Ok(Simple::Pseudo(lower))
            }
        }
        Token::Function(name) => {
            let lower = name.to_ascii_lowercase();
            match lower.as_str() {
                "is" | "matches" | "-webkit-any" | "-moz-any" => {
                    Ok(Simple::Is(p.parse_nested_block(forgiving_in)?))
                }
                "where" => Ok(Simple::Where(p.parse_nested_block(forgiving_in)?)),
                "not" => Ok(Simple::Not(p.parse_nested_block(|p| list_in(p, false))?)),
                "has" => Ok(Simple::Has(p.parse_nested_block(|p| list_in(p, true))?)),
                "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                    let kind = match lower.as_str() {
                        "nth-child" => NthKind::Child,
                        "nth-last-child" => NthKind::LastChild,
                        "nth-of-type" => NthKind::OfType,
                        _ => NthKind::LastOfType,
                    };
                    p.parse_nested_block(|p| {
                        let (a, b) = parse_nth(p)?;
                        let of = if matches!(kind, NthKind::Child | NthKind::LastChild) {
                            p.skip_whitespace();
                            if p.try_parse(|p| p.expect_ident_matching("of")).is_ok() {
                                Some(list_in(p, false)?)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        Ok(Simple::Nth { kind, a, b, of })
                    })
                }
                _ => {
                    let args = p.parse_nested_block(raw_args)?;
                    Ok(Simple::OtherFunction { name: lower, args })
                }
            }
        }
        _ => Err(cssparser::ParseError::custom(())),
    }
}

// ----- matching ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl Tri {
    fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::No, _) | (_, Tri::No) => Tri::No,
            (Tri::Yes, Tri::Yes) => Tri::Yes,
            _ => Tri::Unknown,
        }
    }
    fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::Yes, _) | (_, Tri::Yes) => Tri::Yes,
            (Tri::No, Tri::No) => Tri::No,
            _ => Tri::Unknown,
        }
    }
    fn not(self) -> Tri {
        match self {
            Tri::Yes => Tri::No,
            Tri::No => Tri::Yes,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

impl From<bool> for Tri {
    fn from(value: bool) -> Self {
        if value { Tri::Yes } else { Tri::No }
    }
}

/// Element tree the matcher walks. Nodes are opaque ids.
pub trait Dom {
    fn tag(&self, node: usize) -> &str;
    fn attr(&self, node: usize, name: &str) -> Option<&str>;
    fn parent(&self, node: usize) -> Option<usize>;
    /// Element children in document order.
    fn element_children(&self, node: usize) -> &[usize];
    /// True when the element has no element children and no non-whitespace text.
    fn is_empty(&self, node: usize) -> bool;
    /// State-dependent pseudo-classes (`hover`, `checked`, ...). `Unknown` if undecidable.
    fn state(&self, node: usize, name: &str) -> Tri;
}

fn siblings<D: Dom + ?Sized>(dom: &D, node: usize) -> Option<(&[usize], usize)> {
    let parent = dom.parent(node)?;
    let list = dom.element_children(parent);
    let index = list.iter().position(|&n| n == node)?;
    Some((list, index))
}

fn nth_matches(a: i32, b: i32, position: i64) -> bool {
    // position is 1-based. Find n >= 0 with a*n + b == position.
    let (a, b) = (i64::from(a), i64::from(b));
    if a == 0 {
        return position == b;
    }
    let diff = position - b;
    diff % a == 0 && diff / a >= 0
}

fn simple_match<D: Dom + ?Sized>(simple: &Simple, dom: &D, node: usize) -> Tri {
    match simple {
        Simple::Type(name) => dom.tag(node).eq_ignore_ascii_case(name).into(),
        Simple::Universal => Tri::Yes,
        Simple::Class(name) => dom
            .attr(node, "class")
            .is_some_and(|c| c.split_ascii_whitespace().any(|x| x == name))
            .into(),
        Simple::Id(name) => (dom.attr(node, "id") == Some(name.as_str())).into(),
        Simple::Attr {
            name,
            op,
            value,
            insensitive,
        } => {
            let Some(actual) = dom.attr(node, name) else {
                return Tri::No;
            };
            let fold = |s: &str| {
                if *insensitive {
                    s.to_ascii_lowercase()
                } else {
                    s.to_string()
                }
            };
            let (actual, value) = (fold(actual), fold(value));
            match op {
                AttrOp::Exists => true,
                AttrOp::Equals => actual == value,
                AttrOp::Includes => {
                    !value.is_empty() && actual.split_ascii_whitespace().any(|x| x == value)
                }
                AttrOp::Dash => actual == value || actual.starts_with(&format!("{value}-")),
                AttrOp::Prefix => !value.is_empty() && actual.starts_with(&value),
                AttrOp::Suffix => !value.is_empty() && actual.ends_with(&value),
                AttrOp::Substring => !value.is_empty() && actual.contains(&value),
            }
            .into()
        }
        Simple::Is(list) | Simple::Where(list) => list_match(list, dom, node),
        Simple::Not(list) => list_match(list, dom, node).not(),
        Simple::Has(list) => has_match(list, dom, node),
        Simple::Nth { kind, a, b, of } => {
            let Some((list, index)) = siblings(dom, node) else {
                return Tri::No; // the root element is never an nth-child
            };
            let mut candidates: Vec<usize> = Vec::new();
            let mut unknown = false;
            for &sibling in list {
                let keep = match kind {
                    NthKind::OfType | NthKind::LastOfType => {
                        dom.tag(sibling).eq_ignore_ascii_case(dom.tag(node)).into()
                    }
                    _ => match of {
                        Some(of) => list_match(of, dom, sibling),
                        None => Tri::Yes,
                    },
                };
                match keep {
                    Tri::Yes => candidates.push(sibling),
                    Tri::Unknown => {
                        unknown = true;
                        candidates.push(sibling);
                    }
                    Tri::No => {}
                }
            }
            let Some(pos) = candidates.iter().position(|&n| n == node) else {
                return Tri::No;
            };
            let _ = index;
            if unknown {
                return Tri::Unknown;
            }
            let position = match kind {
                NthKind::Child | NthKind::OfType => pos as i64 + 1,
                NthKind::LastChild | NthKind::LastOfType => (candidates.len() - pos) as i64,
            };
            nth_matches(*a, *b, position).into()
        }
        Simple::Pseudo(name) => pseudo_class(name, dom, node),
        // Shadow-tree selectors never match anything in a light-DOM document stylesheet.
        Simple::OtherFunction { name, .. } if matches!(name.as_str(), "host" | "host-context") => {
            Tri::No
        }
        Simple::OtherFunction { .. } | Simple::Nesting => Tri::Unknown,
        // Handled by the caller: the pseudo-element picks which style map is addressed.
        Simple::PseudoElement { .. } => Tri::Yes,
    }
}

fn pseudo_class<D: Dom + ?Sized>(name: &str, dom: &D, node: usize) -> Tri {
    let is_first = || siblings(dom, node).map(|(_, i)| i == 0);
    let is_last = || siblings(dom, node).map(|(l, i)| i + 1 == l.len());
    let of_type = |from_end: bool| {
        siblings(dom, node).map(|(list, i)| {
            let same = |&&n: &&usize| dom.tag(n).eq_ignore_ascii_case(dom.tag(node));
            if from_end {
                list[i + 1..].iter().filter(same).count() == 0
            } else {
                list[..i].iter().filter(same).count() == 0
            }
        })
    };
    match name {
        "root" | "scope" => dom.parent(node).is_none().into(),
        "first-child" => is_first().unwrap_or(false).into(),
        "last-child" => is_last().unwrap_or(false).into(),
        "only-child" => (is_first().unwrap_or(false) && is_last().unwrap_or(false)).into(),
        "first-of-type" => of_type(false).unwrap_or(false).into(),
        "last-of-type" => of_type(true).unwrap_or(false).into(),
        "only-of-type" => {
            (of_type(false).unwrap_or(false) && of_type(true).unwrap_or(false)).into()
        }
        "empty" => dom.is_empty(node).into(),
        "defined" => Tri::Yes,
        "any-link" | "link" => {
            (matches!(dom.tag(node), "a" | "area") && dom.attr(node, "href").is_some()).into()
        }
        // Pointer and focus state is the DOM's to decide: a resting snapshot says No, the
        // style translator forces one state on one element at a time.
        "hover" | "active" | "focus" | "focus-visible" | "focus-within" => dom.state(node, name),
        // Never true in a captured document.
        "host" | "visited" | "target" | "target-within" | "local-link" | "modal"
        | "popover-open" | "fullscreen" | "-webkit-autofill" | "user-invalid" | "user-valid" => {
            Tri::No
        }
        other => dom.state(node, other),
    }
}

fn has_match<D: Dom + ?Sized>(list: &SelectorList, dom: &D, node: usize) -> Tri {
    let mut result = Tri::No;
    for relative in &list.0 {
        result = result.or(has_one(relative, dom, node));
        if result == Tri::Yes {
            break;
        }
    }
    result
}

fn all_descendants<D: Dom + ?Sized>(dom: &D, node: usize, out: &mut Vec<usize>) {
    // Iterative: captured trees can be thousands of levels deep.
    let mut stack: Vec<usize> = dom.element_children(node).iter().rev().copied().collect();
    while let Some(next) = stack.pop() {
        out.push(next);
        stack.extend(dom.element_children(next).iter().rev());
    }
}

fn has_one<D: Dom + ?Sized>(relative: &Complex, dom: &D, anchor: usize) -> Tri {
    let lead = relative.leading.unwrap_or(Combinator::Descendant);
    let mut candidates: Vec<usize> = Vec::new();
    match lead {
        Combinator::Child => candidates.extend(dom.element_children(anchor)),
        Combinator::Descendant => all_descendants(dom, anchor, &mut candidates),
        Combinator::NextSibling => {
            if let Some((list, i)) = siblings(dom, anchor) {
                candidates.extend(list.get(i + 1));
            }
        }
        Combinator::LaterSibling => {
            if let Some((list, i)) = siblings(dom, anchor) {
                candidates.extend(&list[i + 1..]);
            }
        }
    }
    // The candidate must match the whole relative selector read right to left with the
    // anchor standing for the implicit leading compound. Anchoring is checked by walking
    // back to `anchor` through the combinator chain.
    let mut result = Tri::No;
    for candidate in candidates {
        let r = match_relative(relative, dom, candidate, anchor);
        result = result.or(r);
        if result == Tri::Yes {
            return Tri::Yes;
        }
    }
    result
}

fn match_relative<D: Dom + ?Sized>(rel: &Complex, dom: &D, node: usize, anchor: usize) -> Tri {
    // Match the last compound at `node`, then walk left; when the chain is exhausted the
    // node reached through the leading combinator must be the anchor.
    fn go<D: Dom + ?Sized>(
        rel: &Complex,
        dom: &D,
        index: usize,
        node: usize,
        anchor: usize,
    ) -> Tri {
        let here = compound_match(&rel.compounds[index], dom, node);
        if here == Tri::No {
            return Tri::No;
        }
        let rest = if index == 0 {
            let lead = rel.leading.unwrap_or(Combinator::Descendant);
            walk(lead, dom, node, &mut |candidate| {
                (candidate == anchor).into()
            })
        } else {
            walk(rel.combinators[index - 1], dom, node, &mut |candidate| {
                go(rel, dom, index - 1, candidate, anchor)
            })
        };
        here.and(rest)
    }
    go(rel, dom, rel.compounds.len() - 1, node, anchor)
}

/// Visit the nodes reachable from `node` through `combinator` (towards the left side).
fn walk<D: Dom + ?Sized>(
    combinator: Combinator,
    dom: &D,
    node: usize,
    test: &mut dyn FnMut(usize) -> Tri,
) -> Tri {
    let mut result = Tri::No;
    match combinator {
        Combinator::Child => {
            if let Some(parent) = dom.parent(node) {
                result = test(parent);
            }
        }
        Combinator::Descendant => {
            let mut current = dom.parent(node);
            while let Some(ancestor) = current {
                result = result.or(test(ancestor));
                if result == Tri::Yes {
                    return Tri::Yes;
                }
                current = dom.parent(ancestor);
            }
        }
        Combinator::NextSibling => {
            if let Some((list, i)) = siblings(dom, node)
                && i > 0
            {
                result = test(list[i - 1]);
            }
        }
        Combinator::LaterSibling => {
            if let Some((list, i)) = siblings(dom, node) {
                for &sibling in &list[..i] {
                    result = result.or(test(sibling));
                    if result == Tri::Yes {
                        return Tri::Yes;
                    }
                }
            }
        }
    }
    result
}

fn compound_match<D: Dom + ?Sized>(compound: &Compound, dom: &D, node: usize) -> Tri {
    let mut result = Tri::Yes;
    for simple in &compound.simples {
        result = result.and(simple_match(simple, dom, node));
        if result == Tri::No {
            break;
        }
    }
    result
}

pub fn list_match<D: Dom + ?Sized>(list: &SelectorList, dom: &D, node: usize) -> Tri {
    let mut result = Tri::No;
    for complex in &list.0 {
        result = result.or(complex_match(complex, dom, node));
        if result == Tri::Yes {
            break;
        }
    }
    result
}

/// Match a complex selector against an element. The pseudo-element (if any) is ignored
/// here; callers pick the style map with [`Complex::pseudo_element`].
pub fn complex_match<D: Dom + ?Sized>(complex: &Complex, dom: &D, node: usize) -> Tri {
    // Results are memoized per (compound index, node): without it a chain of descendant
    // combinators over a deep tree backtracks exponentially.
    fn go<D: Dom + ?Sized>(
        complex: &Complex,
        dom: &D,
        index: usize,
        node: usize,
        memo: &std::cell::RefCell<std::collections::HashMap<(usize, usize), Tri>>,
    ) -> Tri {
        if let Some(&known) = memo.borrow().get(&(index, node)) {
            return known;
        }
        let here = compound_match(&complex.compounds[index], dom, node);
        let result = if here == Tri::No || index == 0 {
            here
        } else {
            here.and(walk(
                complex.combinators[index - 1],
                dom,
                node,
                &mut |candidate| go(complex, dom, index - 1, candidate, memo),
            ))
        };
        memo.borrow_mut().insert((index, node), result);
        result
    }
    if complex.leading.is_some() {
        return Tri::Unknown;
    }
    let memo = std::cell::RefCell::new(std::collections::HashMap::new());
    go(complex, dom, complex.compounds.len() - 1, node, &memo)
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestNode = (
        &'static str,
        Vec<(&'static str, &'static str)>,
        Option<usize>,
    );

    struct Tree {
        // (tag, attrs, parent)
        nodes: Vec<TestNode>,
        children: Vec<Vec<usize>>,
    }

    impl Tree {
        fn new(nodes: Vec<TestNode>) -> Self {
            let mut children = vec![Vec::new(); nodes.len()];
            for (i, (_, _, parent)) in nodes.iter().enumerate() {
                if let Some(p) = parent {
                    children[*p].push(i);
                }
            }
            Tree { nodes, children }
        }
    }

    impl Dom for Tree {
        fn tag(&self, n: usize) -> &str {
            self.nodes[n].0
        }
        fn attr(&self, n: usize, name: &str) -> Option<&str> {
            self.nodes[n]
                .1
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
        }
        fn parent(&self, n: usize) -> Option<usize> {
            self.nodes[n].2
        }
        fn element_children(&self, n: usize) -> &[usize] {
            &self.children[n]
        }
        fn is_empty(&self, n: usize) -> bool {
            self.children[n].is_empty()
        }
        fn state(&self, _: usize, name: &str) -> Tri {
            match name {
                "hover" | "active" | "focus" | "focus-visible" | "focus-within" => Tri::No,
                _ => Tri::Unknown,
            }
        }
    }

    fn tree() -> Tree {
        Tree::new(vec![
            ("html", vec![], None),
            ("body", vec![("class", "page dark")], Some(0)),
            ("ul", vec![("id", "menu")], Some(1)),
            ("li", vec![("class", "item")], Some(2)),
            (
                "li",
                vec![("class", "item active"), ("data-x", "a-b")],
                Some(2),
            ),
            ("li", vec![("class", "item")], Some(2)),
            ("a", vec![("href", "/x")], Some(4)),
        ])
    }

    fn m(selector: &str, node: usize) -> Tri {
        let list = parse_list(selector).unwrap();
        list_match(&list, &tree(), node)
    }

    #[test]
    fn specificity_follows_the_spec() {
        let spec = |s: &str| parse_list(s).unwrap().0[0].specificity();
        assert_eq!(spec("#a .b c"), (1, 1, 1));
        assert_eq!(spec(":is(.a, #b) > i"), (1, 0, 1));
        assert_eq!(spec(":where(#a) .b"), (0, 1, 0));
        assert_eq!(spec("li:nth-child(2n+1)"), (0, 1, 1));
        assert_eq!(spec("a::before"), (0, 0, 2));
        assert_eq!(spec("[data-x]:not(.q)"), (0, 2, 0));
    }

    #[test]
    fn combinators_and_structural_pseudos() {
        assert_eq!(m("ul > li.active", 4), Tri::Yes);
        assert_eq!(m("li.active + li", 5), Tri::Yes);
        assert_eq!(m("li.active ~ li", 3), Tri::No);
        assert_eq!(m("body li", 3), Tri::Yes);
        assert_eq!(m("li:first-child", 3), Tri::Yes);
        assert_eq!(m("li:last-child", 5), Tri::Yes);
        assert_eq!(m("li:nth-child(2)", 4), Tri::Yes);
        assert_eq!(m("li:nth-child(odd)", 5), Tri::Yes);
        assert_eq!(m("li:nth-last-child(1)", 5), Tri::Yes);
        assert_eq!(m(":root", 0), Tri::Yes);
        assert_eq!(m("li:not(.active)", 4), Tri::No);
        assert_eq!(m("li:has(> a[href])", 4), Tri::Yes);
        assert_eq!(m("li:has(a)", 3), Tri::No);
        assert_eq!(m("[data-x|=\"a\"]", 4), Tri::Yes);
        assert_eq!(m("#menu .item.active", 4), Tri::Yes);
    }

    #[test]
    fn deep_descendant_chains_do_not_backtrack_exponentially() {
        let mut nodes = vec![("html", vec![], None)];
        for i in 0..60 {
            nodes.push(("div", vec![("class", "a")], Some(i)));
        }
        let tree = Tree::new(nodes);
        let list = parse_list(".a .a .a .a .a .a .a .a .a .a .a .a .zzz").unwrap();
        let started = std::time::Instant::now();
        assert_eq!(list_match(&list, &tree, 60), Tri::No);
        assert!(started.elapsed().as_secs() < 5);
    }

    #[test]
    fn interactive_states_are_resting_and_unknown_is_explicit() {
        assert_eq!(m("li:hover", 3), Tri::No);
        assert_eq!(m("li:focus-visible", 3), Tri::No);
        assert_eq!(m("li:lang(en)", 3), Tri::Unknown);
        assert_eq!(m(":host", 0), Tri::No);
        assert_eq!(m(":host(.x)", 0), Tri::No);
        assert_eq!(m(":root, :host", 0), Tri::Yes);
        assert_eq!(m("li:not(:lang(en))", 3), Tri::Unknown);
        assert_eq!(m("li:hover:lang(en)", 3), Tri::No);
        assert_eq!(m("a:any-link", 6), Tri::Yes);
    }

    #[test]
    fn round_trips_and_rejects_garbage() {
        for s in [
            ".a > .b ~ c",
            "a:not(.b, #c)::before",
            "li:nth-child(2n+1 of .x)",
            "&:hover",
            "> .x",
        ] {
            let first = parse_list(s).unwrap().to_css();
            assert_eq!(parse_list(&first).unwrap().to_css(), first, "{s}");
        }
        assert!(parse_list(".a >").is_err());
        assert!(parse_list("").is_err());
        assert!(parse_list("a,,b").is_err());
    }

    #[test]
    fn pseudo_element_is_exposed_separately() {
        let list = parse_list("p::before, p:after").unwrap();
        assert_eq!(list.0[0].pseudo_element(), Some("before"));
        assert_eq!(list.0[1].pseudo_element(), Some("after"));
    }
}
