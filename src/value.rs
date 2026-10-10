use crate::{Error, ErrorKind, Navigator, Result};

/// An XPath 1.0 value. Node sets supplied by callers need not be sorted.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<N> {
    Nodes(Vec<N>),
    Boolean(bool),
    Number(f64),
    String(String),
}
impl<N> Value<N> {
    pub fn to_boolean(&self) -> bool {
        match self {
            Self::Nodes(nodes) => !nodes.is_empty(),
            Self::Boolean(value) => *value,
            Self::Number(value) => *value != 0.0 && !value.is_nan(),
            Self::String(value) => !value.is_empty(),
        }
    }
    pub fn to_number<T: Navigator<Node = N>>(&self, nav: &T) -> Result<f64> {
        Ok(match self {
            Self::Number(value) => *value,
            Self::Boolean(value) => {
                if *value {
                    1.0
                } else {
                    0.0
                }
            }
            Self::String(value) => string_to_number(value),
            Self::Nodes(_) => string_to_number(&self.to_string(nav)?),
        })
    }
    pub fn to_string<T: Navigator<Node = N>>(&self, nav: &T) -> Result<String> {
        match self {
            Self::String(value) => Ok(value.clone()),
            Self::Number(value) => Ok(number_to_string(*value)),
            Self::Boolean(value) => Ok(if *value { "true" } else { "false" }.into()),
            Self::Nodes(nodes) => match first_node(nodes, nav)? {
                Some(node) => nav.string_value(node),
                None => Ok(String::new()),
            },
        }
    }
    pub fn as_nodes(&self) -> Result<&[N]> {
        match self {
            Self::Nodes(nodes) => Ok(nodes),
            _ => Err(Error::new(ErrorKind::Type, "expected a node set")),
        }
    }

    pub(crate) fn into_nodes(self) -> Result<Vec<N>> {
        match self {
            Self::Nodes(nodes) => Ok(nodes),
            _ => Err(Error::new(ErrorKind::Type, "expected a node set")),
        }
    }
}

pub(crate) fn first_node<'a, T: Navigator>(
    nodes: &'a [T::Node],
    nav: &T,
) -> Result<Option<&'a T::Node>> {
    for node in nodes {
        nav.validate_node(node)?;
    }
    Ok(nodes.iter().min_by(|a, b| nav.compare_document_order(a, b)))
}

pub(crate) fn xml_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// XPath permits decimal notation, an optional minus sign, and XML whitespace.
/// Rust's parser also accepts exponents, a plus sign, and special values, so
/// validate the XPath lexical grammar before asking it to round to binary64.
pub(crate) fn string_to_number(value: &str) -> f64 {
    let value = value.trim_matches(xml_whitespace);
    let digits = value.strip_prefix('-').unwrap_or(value);
    let mut seen_digit = false;
    let mut seen_dot = false;
    for byte in digits.bytes() {
        match byte {
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => return f64::NAN,
        }
    }
    if !seen_digit {
        return f64::NAN;
    }
    value.parse().unwrap_or(f64::NAN)
}

pub(crate) fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value == f64::INFINITY {
        "Infinity".into()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".into()
    } else if value == 0.0 {
        "0".into()
    } else if value.fract() == 0.0 {
        // Integral binary64 values require their exact integer decimal.
        format!("{value:.0}")
    }
    // Fractional values use the shortest round-tripping decimal.
    else {
        value.to_string()
    }
}

/// Nearest integer, with ties toward positive infinity. Adding 0.5 first
/// introduces an extra rounding step for numbers just below a half-integer.
pub(crate) fn xpath_round(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if (-0.5..0.0).contains(&value) {
        return -0.0;
    }
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_lexical_grammar_uses_only_xml_whitespace_and_ascii_digits() {
        for (s, expected) in [
            ("  \r\n\t-2.5\t", -2.5),
            (".5", 0.5),
            ("5.", 5.0),
            ("0002", 2.0),
        ] {
            assert_eq!(string_to_number(s), expected, "{s:?}");
        }
        for s in [
            "",
            " \t",
            "+2",
            "1e3",
            "Infinity",
            "-Infinity",
            "NaN",
            ".",
            "-",
            "1 2",
            "\u{a0}2",
            "２",
            "0x2",
        ] {
            assert!(string_to_number(s).is_nan(), "{s:?}");
        }
        assert!(string_to_number("-0").is_sign_negative());
        assert!(string_to_number(&"9".repeat(400)).is_infinite());
    }

    #[test]
    fn number_string_specials_and_decimal_notation() {
        for (n, expected) in [
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (-0.0, "0"),
            (1.0, "1"),
            (1.25, "1.25"),
            (1e-8, "0.00000001"),
            (1e20, "100000000000000000000"),
        ] {
            assert_eq!(number_to_string(n), expected);
        }
        for n in [
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::MAX,
            -f64::MAX,
            0.0000000000000012345,
        ] {
            let s = number_to_string(n);
            assert!(!s.contains(['e', 'E']), "{s}");
            assert_eq!(string_to_number(&s), n);
        }
    }

    #[test]
    fn integral_numbers_render_the_exact_integer_decimal() {
        for (value, expected) in [
            (1000000000000000128.0, "1000000000000000128"),
            (-1000000000000000128.0, "-1000000000000000128"),
            (2f64.powi(80), "1208925819614629174706176"),
            (1e21, "1000000000000000000000"),
        ] {
            assert_eq!(number_to_string(value), expected);
        }
    }

    #[test]
    fn xpath_round_ties_toward_positive_infinity_and_preserves_zero() {
        for (n, expected) in [
            (-1.5, -1.0),
            (1.5, 2.0),
            (-1.51, -2.0),
            (0.49999999999999994, 0.0),
            (4503599627370497.0, 4503599627370497.0),
        ] {
            assert_eq!(xpath_round(n), expected);
        }
        for n in [-0.5, -0.1, -0.0] {
            assert_eq!(xpath_round(n), 0.0);
            assert!(xpath_round(n).is_sign_negative());
        }
        assert!(xpath_round(f64::NAN).is_nan());
        assert_eq!(xpath_round(f64::INFINITY), f64::INFINITY);
        assert_eq!(xpath_round(f64::NEG_INFINITY), f64::NEG_INFINITY);
    }

    #[test]
    fn boolean_conversions_follow_xpath_truth_rules() {
        for value in [
            Value::<u8>::Nodes(vec![]),
            Value::String(String::new()),
            Value::Number(0.0),
            Value::Number(-0.0),
            Value::Number(f64::NAN),
            Value::Boolean(false),
        ] {
            assert!(!value.to_boolean());
        }
        for value in [
            Value::Nodes(vec![1u8]),
            Value::String("false".into()),
            Value::Number(f64::INFINITY),
            Value::Number(-1.0),
            Value::Boolean(true),
        ] {
            assert!(value.to_boolean());
        }
    }
}
