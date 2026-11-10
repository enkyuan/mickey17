//! Small TypeScript text helpers shared by the emitters.
use crate::semantic::components::{camel, pascal};
use crate::semantic::js::{Ex, Lit, StateId};
use std::collections::HashMap;

pub use super::jsx::ts_string;

pub fn kebab(s: &str) -> String {
    crate::semantic::components::words(s).join("-")
}

pub fn ident(s: &str) -> String {
    camel(s)
}

pub fn type_name(s: &str) -> String {
    pascal(s)
}

/// TypeScript source for a summarised expression; `None` when it is not expressible
/// from the summary (the caller then reports a gap instead of guessing).
pub fn ex_ts(ex: &Ex, names: &HashMap<StateId, String>) -> Option<String> {
    Some(match ex {
        Ex::Lit { value } => match value {
            Lit::Bool(b) => b.to_string(),
            Lit::Num(n) => n.to_string(),
            Lit::Str(s) => ts_string(s),
            Lit::Null => "null".into(),
            Lit::Undefined => "undefined".into(),
        },
        Ex::State { state } => names.get(state)?.clone(),
        Ex::Not { arg } => format!("!{}", paren(arg, names)?),
        Ex::Cond {
            test,
            then,
            otherwise,
        } => format!(
            "{} ? {} : {}",
            paren(test, names)?,
            paren(then, names)?,
            paren(otherwise, names)?
        ),
        Ex::Eq { left, right } => format!("{} === {}", paren(left, names)?, paren(right, names)?),
        _ => return None,
    })
}

fn paren(ex: &Ex, names: &HashMap<StateId, String>) -> Option<String> {
    let s = ex_ts(ex, names)?;
    Some(if matches!(ex, Ex::Lit { .. } | Ex::State { .. }) {
        s
    } else {
        format!("({s})")
    })
}

/// Join name fragments into one PascalCase identifier. A fragment that is already a
/// PascalCase identifier (`APIExplorer`) is kept verbatim; words already present are
/// dropped (`Investors` + `Filter investors` -> `InvestorsFilter`), and a single word that
/// is a strict prefix of the next fragment's first word is absorbed (`Nav` + `NavigationMenu`).
pub fn join_words(parts: &[&str]) -> String {
    use crate::semantic::components::{identifier_or_pascal, words};
    let mut seen: Vec<String> = Vec::new();
    let mut units: Vec<(String, Vec<String>)> = Vec::new();
    for p in parts {
        let ws = words(p);
        let fresh: Vec<String> = ws.iter().filter(|w| !seen.contains(w)).cloned().collect();
        if fresh.is_empty() {
            continue;
        }
        let text = if fresh.len() == ws.len() {
            identifier_or_pascal(p)
        } else {
            fresh.iter().map(|w| pascal(w)).collect()
        };
        seen.extend(fresh.iter().cloned());
        units.push((text, fresh));
    }
    // Absorb a one-word unit into a following unit that starts with that word.
    let mut i = 0;
    while i + 1 < units.len() {
        if units[i].1.len() == 1
            && units[i + 1]
                .1
                .first()
                .is_some_and(|n| n.starts_with(&units[i].1[0]) && n.len() > units[i].1[0].len())
        {
            units.remove(i);
        } else {
            i += 1;
        }
    }
    units.into_iter().map(|u| u.0).collect()
}
