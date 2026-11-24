//! Static text that every encoded primary key value starts with or equals.

/// A static guarantee about one encoded string key, or a discriminator over it.
///
/// `Any` promises nothing and matches every value. Composite key templates
/// derive `Prefix` from their leading literal and `Exact` when the template is
/// literal-only; plain field keys are `Any`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPattern {
    Any,
    Prefix(&'static str),
    Exact(&'static str),
}

/// Patterns for the physical partition key and, when declared, the sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPatterns {
    pub partition: KeyPattern,
    pub sort: KeyPattern,
}
impl KeyPatterns {
    /// No static guarantee about either key.
    pub const ANY: Self = Self {
        partition: KeyPattern::Any,
        sort: KeyPattern::Any,
    };
}

impl KeyPattern {
    /// True when every string matched by `self` is also matched by `other`.
    pub const fn implies(self, other: Self) -> bool {
        match (self, other) {
            (_, Self::Any) => true,
            (Self::Prefix(a), Self::Prefix(b)) | (Self::Exact(a), Self::Prefix(b)) => {
                starts_with(a, b)
            }
            (Self::Exact(a), Self::Exact(b)) => same(a, b),
            _ => false,
        }
    }

    /// True when at least one string can be matched by both patterns.
    pub const fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Self::Any, _) | (_, Self::Any) => true,
            (Self::Prefix(a), Self::Prefix(b)) => starts_with(a, b) || starts_with(b, a),
            (Self::Prefix(p), Self::Exact(e)) | (Self::Exact(e), Self::Prefix(p)) => {
                starts_with(e, p)
            }
            (Self::Exact(a), Self::Exact(b)) => same(a, b),
        }
    }

    pub fn matches(self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Prefix(prefix) => value.starts_with(prefix),
            Self::Exact(exact) => value == exact,
        }
    }

    /// The literal text, or `None` for `Any`.
    pub const fn text(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Prefix(text) | Self::Exact(text) => Some(text),
        }
    }
}

pub(crate) const fn starts_with(value: &str, prefix: &str) -> bool {
    let (value, prefix) = (value.as_bytes(), prefix.as_bytes());
    if prefix.len() > value.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if value[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub(crate) const fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && starts_with(a, b)
}

#[cfg(test)]
mod tests {
    use super::KeyPattern::{self, *};

    #[test]
    fn implication_and_overlap_are_exact_over_static_text() {
        let cases: [(KeyPattern, KeyPattern, bool, bool); 10] = [
            (Prefix("ORD#"), Any, true, true),
            (Any, Prefix("ORD#"), false, true),
            (Prefix("ORD#1"), Prefix("ORD#"), true, true),
            (Prefix("ORD#"), Prefix("ORD#1"), false, true),
            (Prefix("ORD#"), Prefix("INV#"), false, false),
            (Exact("META"), Prefix("ME"), true, true),
            (Exact("META"), Prefix("LINE#"), false, false),
            (Exact("META"), Exact("META"), true, true),
            (Exact("META"), Exact("MET"), false, false),
            (Prefix("MET"), Exact("META"), false, true),
        ];
        for (a, b, implies, overlaps) in cases {
            assert_eq!(a.implies(b), implies, "{a:?} implies {b:?}");
            assert_eq!(a.overlaps(b), overlaps, "{a:?} overlaps {b:?}");
            assert_eq!(b.overlaps(a), overlaps, "{b:?} overlaps {a:?}");
        }
        assert!(Prefix("LINE#").matches("LINE#00000000000000000001"));
        assert!(!Prefix("LINE#").matches("LINE"));
        assert!(Exact("META").matches("META") && !Exact("META").matches("META#"));
        assert!(Any.matches(""));
        assert_eq!((Any.text(), Exact("A").text()), (None, Some("A")));
    }
}
