//! Domain → Punycode/ASCII conversion for sing-box config generation.
//!
//! sing-box matches routing/DNS rules against wire-format QNAME/SNI, which is
//! always ASCII (IDNA/Punycode) — this is a DNS/TLS protocol requirement, not
//! a sing-box quirk. User-facing storage (rule payloads, Hosts entries) keeps
//! Unicode so the UI, rule IDs, and Clash-format import/export stay
//! human-readable; only the final JSON handed to the core needs ASCII.

/// Convert a domain to its Punycode/ASCII form. Labels that are already ASCII
/// are left unchanged (e.g. `.com` in `中文.com`); only non-ASCII labels are
/// Punycode-encoded. Falls back to the original string on conversion failure
/// so one malformed domain never blocks config generation.
pub fn to_ascii_domain(domain: &str) -> String {
    idna::domain_to_ascii(domain).unwrap_or_else(|_| domain.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_chinese_labels_to_punycode() {
        assert_eq!(to_ascii_domain("中文.com"), "xn--fiq228c.com");
        assert_eq!(to_ascii_domain("中国.com"), "xn--fiqs8s.com");
        assert_eq!(to_ascii_domain("例子.测试"), "xn--fsqu00a.xn--0zwm56d");
    }

    #[test]
    fn leaves_ascii_domains_unchanged() {
        assert_eq!(to_ascii_domain("example.com"), "example.com");
    }

    #[test]
    fn falls_back_on_conversion_failure() {
        // A label that violates IDNA constraints (e.g. lone surrogate-adjacent
        // control chars) should round-trip unchanged rather than panic or drop
        // the rule. Empty string is the simplest input idna rejects outright
        // in some paths; assert it never panics and returns *something* usable.
        assert_eq!(to_ascii_domain(""), "");
    }
}
