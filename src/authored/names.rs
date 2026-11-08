//! Refuse compiler identifiers while preserving descriptive names and route slugs.
pub(super) fn meaningful(value: &str) -> bool {
    let name = value.split('.').next().unwrap_or(value);
    let lower = name.to_ascii_lowercase();
    let hash = |s: &str| s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if name.is_empty() || hash(name) {
        return false;
    }
    for prefix in ["client", "native", "module", "esm", "component", "chunk"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(['-', '_']);
            if !rest.is_empty() && (rest.bytes().all(|b| b.is_ascii_digit()) || hash(rest)) {
                return false;
            }
        }
    }
    !value.split(['-', '_', '.']).any(hash)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_describe_the_application_instead_of_bundle_identity() {
        for name in [
            "Client72b310ddae8ef28e",
            "native-1234abcd",
            "Component19",
            "Module123",
            "esm-deadbeef",
            "hero-deadbeef",
            "hero.deadbeef.tsx",
            "89abcdef.tsx",
        ] {
            assert!(!meaningful(name), "{name}");
        }
        for name in [
            "ClientLogos",
            "WebGL2Scene",
            "article-2026",
            "hero-background",
            "motion-v4",
            "mai-cyber-1-1-flash",
        ] {
            assert!(meaningful(name), "{name}");
        }
    }
}
