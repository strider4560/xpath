use crate::value::{first_node, string_to_number, xml_whitespace, xpath_round};
use crate::{Error, ErrorKind, FunctionContext, Navigator, NodeKind, Result, Value};
use std::collections::{HashMap, HashSet};

/// Dispatch precisely the XPath 1.0 core library. None lets the evaluator try a
/// caller-provided extension; recognized functions always preserve their errors.
pub(crate) fn call<N: Navigator>(
    name: &str,
    args: &[Value<N::Node>],
    ctx: &FunctionContext<'_, N>,
    remaining: &mut usize,
) -> Option<Result<Value<N::Node>>> {
    let (min, max) = match name {
        "last" | "position" | "true" | "false" => (0, 0),
        "count" | "id" | "boolean" | "not" | "lang" | "sum" | "floor" | "ceiling" | "round" => {
            (1, 1)
        }
        "local-name" | "namespace-uri" | "name" | "string" | "number" | "string-length"
        | "normalize-space" => (0, 1),
        "starts-with" | "contains" | "substring-before" | "substring-after" => (2, 2),
        "substring" => (2, 3),
        "translate" => (3, 3),
        "concat" => (2, usize::MAX),
        _ => return None,
    };
    if args.len() < min || args.len() > max {
        let expectation = if max == usize::MAX {
            format!("at least {min}")
        } else if min == max {
            min.to_string()
        } else {
            format!("{min} to {max}")
        };
        return Some(Err(Error::new(
            ErrorKind::Arity,
            format!("{name} expects {expectation} arguments, got {}", args.len()),
        )));
    }
    Some(evaluate(name, args, ctx, remaining))
}

fn evaluate<N: Navigator>(
    name: &str,
    args: &[Value<N::Node>],
    ctx: &FunctionContext<'_, N>,
    remaining: &mut usize,
) -> Result<Value<N::Node>> {
    let nav = ctx.navigator;
    let string = |index: usize| args[index].to_string(nav);
    let number = |index: usize| args[index].to_number(nav);
    let context_string = || match args.first() {
        Some(value) => value.to_string(nav),
        None => nav.string_value(ctx.node),
    };
    Ok(match name {
        "last" => Value::Number(ctx.size as f64),
        "position" => Value::Number(ctx.position as f64),
        "count" => Value::Number(args[0].as_nodes()?.len() as f64),
        "id" => {
            let strings = match &args[0] {
                Value::Nodes(nodes) => nodes
                    .iter()
                    .map(|node| {
                        charge_navigation(remaining)?;
                        nav.string_value(node)
                    })
                    .collect::<Result<Vec<_>>>()?,
                value => vec![value.to_string(nav)?],
            };
            let mut nodes = Vec::new();
            let mut seen = HashSet::new();
            for value in strings {
                for id in value.split(xml_whitespace).filter(|id| !id.is_empty()) {
                    charge_navigation(remaining)?;
                    if let Some(node) = nav.element_by_id(ctx.node, id)? {
                        if seen.insert(node.clone()) {
                            nodes.push(node);
                        }
                    }
                }
            }
            nodes.sort_by(|a, b| nav.compare_document_order(a, b));
            Value::Nodes(nodes)
        }
        "name" | "local-name" | "namespace-uri" => {
            let node = match args.first() {
                Some(value) => first_node(value.as_nodes()?, nav)?,
                None => Some(ctx.node),
            };
            let result = match node {
                None => String::new(),
                Some(node) => {
                    let kind = nav.kind(node);
                    match (name, kind) {
                        (_, NodeKind::Root | NodeKind::Text | NodeKind::Comment) => String::new(),
                        (
                            "namespace-uri",
                            NodeKind::Namespace | NodeKind::ProcessingInstruction,
                        ) => String::new(),
                        ("name", _) => nav.qualified_name(node)?,
                        ("local-name", _) => nav
                            .name(node)?
                            .map(|name| name.local_name)
                            .unwrap_or_default(),
                        _ => nav
                            .name(node)?
                            .map(|name| name.namespace_uri)
                            .unwrap_or_default(),
                    }
                }
            };
            Value::String(result)
        }
        "string" => Value::String(context_string()?),
        "concat" => {
            let mut result = String::new();
            for value in args {
                result.push_str(&value.to_string(nav)?);
            }
            Value::String(result)
        }
        "starts-with" => Value::Boolean(string(0)?.starts_with(&string(1)?)),
        "contains" => Value::Boolean(string(0)?.contains(&string(1)?)),
        "substring-before" | "substring-after" => {
            let haystack = string(0)?;
            let needle = string(1)?;
            Value::String(match haystack.find(&needle) {
                Some(index) if name == "substring-before" => haystack[..index].into(),
                Some(index) => haystack[index + needle.len()..].into(),
                None => String::new(),
            })
        }
        "substring" => {
            let value = string(0)?;
            let start = xpath_round(number(1)?);
            // Without a length, the condition is only p >= round(start).
            // With a length, IEEE addition is required: -Inf + Inf is NaN.
            let end = if args.len() == 3 {
                Some(start + xpath_round(number(2)?))
            } else {
                None
            };
            Value::String(
                value
                    .chars()
                    .enumerate()
                    .filter_map(|(index, c)| {
                        let position = (index + 1) as f64;
                        (position >= start && end.is_none_or(|end| position < end)).then_some(c)
                    })
                    .collect(),
            )
        }
        "string-length" => Value::Number(context_string()?.chars().count() as f64),
        "normalize-space" => Value::String(
            context_string()?
                .split(xml_whitespace)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        "translate" => {
            let value = string(0)?;
            let from = string(1)?;
            let to: Vec<char> = string(2)?.chars().collect();
            let mut mappings = HashMap::new();
            for (index, c) in from.chars().enumerate() {
                mappings.entry(c).or_insert_with(|| to.get(index).copied());
            }
            Value::String(
                value
                    .chars()
                    .filter_map(|c| mappings.get(&c).copied().unwrap_or(Some(c)))
                    .collect(),
            )
        }
        "boolean" => Value::Boolean(args[0].to_boolean()),
        "not" => Value::Boolean(!args[0].to_boolean()),
        "true" => Value::Boolean(true),
        "false" => Value::Boolean(false),
        "lang" => Value::Boolean(language_matches(&string(0)?, ctx, remaining)?),
        "number" => Value::Number(match args.first() {
            Some(value) => value.to_number(nav)?,
            None => string_to_number(&nav.string_value(ctx.node)?),
        }),
        "sum" => {
            let mut sum = 0.0;
            for node in args[0].as_nodes()? {
                charge_navigation(remaining)?;
                sum += string_to_number(&nav.string_value(node)?);
            }
            Value::Number(sum)
        }
        "floor" => Value::Number(number(0)?.floor()),
        "ceiling" => Value::Number(number(0)?.ceil()),
        "round" => Value::Number(xpath_round(number(0)?)),
        _ => unreachable!("arity dispatch recognizes only the core library"),
    })
}

fn charge_navigation(remaining: &mut usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| Error::new(ErrorKind::Limit, "evaluation operation budget exceeded"))?;
    Ok(())
}

fn language_matches<N: Navigator>(
    wanted: &str,
    ctx: &FunctionContext<'_, N>,
    remaining: &mut usize,
) -> Result<bool> {
    let nav = ctx.navigator;
    let mut node = Some(ctx.node.clone());
    while let Some(current) = node {
        if nav.kind(&current) == NodeKind::Element {
            for attribute in nav.attributes(&current) {
                charge_navigation(remaining)?;
                let attribute = attribute?;
                if nav.name(&attribute)?.is_some_and(|name| {
                    name.namespace_uri == "http://www.w3.org/XML/1998/namespace"
                        && name.local_name == "lang"
                }) {
                    let actual = nav.string_value(&attribute)?;
                    return Ok(actual.eq_ignore_ascii_case(wanted)
                        || (actual
                            .get(..wanted.len())
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(wanted))
                            && actual.as_bytes().get(wanted.len()) == Some(&b'-')));
                }
            }
        }
        charge_navigation(remaining)?;
        node = nav.parent(&current)?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, ErrorKind, ExpandedName, NodeKind};
    use std::cmp::Ordering;

    // A fixed application-owned tree: root -> p:outer -> inner -> text.
    // Outer carries xml:lang="en-US"; inner may carry an override.
    struct Tree {
        inner_language: Option<&'static str>,
    }
    impl Navigator for Tree {
        type Node = u8;
        fn kind(&self, n: &u8) -> NodeKind {
            match n {
                0 => NodeKind::Root,
                1 | 3 | 7 => NodeKind::Element,
                2 | 4 => NodeKind::Attribute,
                5 => NodeKind::Namespace,
                6 => NodeKind::ProcessingInstruction,
                _ => NodeKind::Text,
            }
        }
        fn parent(&self, n: &u8) -> Result<Option<u8>> {
            Ok(match n {
                0 => None,
                1 => Some(0),
                2 | 3 | 5 | 6 | 7 => Some(1),
                _ => Some(3),
            })
        }
        fn children<'a>(&'a self, n: &'a u8) -> impl Iterator<Item = Result<u8>> + 'a {
            let nodes = match n {
                0 => vec![1],
                1 => vec![3, 6, 7],
                3 => vec![8],
                _ => vec![],
            };
            nodes.into_iter().map(Ok)
        }
        fn attributes<'a>(&'a self, n: &'a u8) -> impl Iterator<Item = Result<u8>> + 'a {
            let nodes = match n {
                1 => vec![2],
                3 if self.inner_language.is_some() => vec![4],
                _ => vec![],
            };
            nodes.into_iter().map(Ok)
        }
        fn name(&self, n: &u8) -> Result<Option<ExpandedName>> {
            Ok(match n {
                1 => Some(ExpandedName::new("urn:test", "outer")),
                3 | 7 => Some(ExpandedName::local("inner")),
                2 | 4 => Some(ExpandedName::new(
                    "http://www.w3.org/XML/1998/namespace",
                    "lang",
                )),
                5 => Some(ExpandedName::local("p")),
                6 => Some(ExpandedName::local("target")),
                _ => None,
            })
        }
        fn qualified_name(&self, n: &u8) -> Result<String> {
            if *n == 1 {
                Ok("p:outer".into())
            } else {
                Ok(self.name(n)?.map(|n| n.local_name).unwrap_or_default())
            }
        }
        fn string_value(&self, n: &u8) -> Result<String> {
            Ok(match n {
                2 => "en-US",
                4 => self.inner_language.unwrap(),
                5 => "urn:test",
                6 => "data",
                7 => "7",
                9 => " b\tc ",
                99 => return Err(Error::new(ErrorKind::Navigation, "unreadable node")),
                _ => "3",
            }
            .into())
        }
        fn compare_document_order(&self, a: &u8, b: &u8) -> Ordering {
            a.cmp(b)
        }
        fn element_by_id(&self, _: &u8, id: &str) -> Result<Option<u8>> {
            Ok(match id {
                "a" => Some(3),
                "b" => Some(7),
                "c" => Some(1),
                "fail" => return Err(Error::new(ErrorKind::Navigation, "id lookup failed")),
                _ => None,
            })
        }
    }
    fn run(name: &str, args: &[Value<u8>]) -> Result<Value<u8>> {
        run_at(
            name,
            args,
            &Tree {
                inner_language: None,
            },
            8,
        )
    }
    fn run_at(name: &str, args: &[Value<u8>], nav: &Tree, node: u8) -> Result<Value<u8>> {
        let ctx = FunctionContext {
            navigator: nav,
            node: &node,
            initial_node: &0,
            position: 2,
            size: 5,
        };
        call(name, args, &ctx, &mut 100_000).expect("core function missing")
    }
    fn s(s: &str) -> Value<u8> {
        Value::String(s.into())
    }
    fn n(n: f64) -> Value<u8> {
        Value::Number(n)
    }

    struct ValidatingTree(Tree);
    impl Navigator for ValidatingTree {
        type Node = u8;
        fn validate_node(&self, node: &u8) -> Result<()> {
            if *node == 99 {
                Err(Error::new(ErrorKind::Navigation, "foreign handle"))
            } else {
                Ok(())
            }
        }
        fn kind(&self, node: &u8) -> NodeKind {
            self.0.kind(node)
        }
        fn parent(&self, node: &u8) -> Result<Option<u8>> {
            self.0.parent(node)
        }
        fn children<'a>(&'a self, node: &'a u8) -> impl Iterator<Item = Result<u8>> + 'a {
            self.0.children(node)
        }
        fn name(&self, node: &u8) -> Result<Option<ExpandedName>> {
            self.0.name(node)
        }
        fn string_value(&self, node: &u8) -> Result<String> {
            self.0.string_value(node)
        }
        fn compare_document_order(&self, a: &u8, b: &u8) -> Ordering {
            assert!(*a != 99 && *b != 99, "foreign node reached comparison");
            self.0.compare_document_order(a, b)
        }
    }

    #[test]
    fn conversions_and_names_validate_every_handle_before_document_order_comparison() {
        let nav = ValidatingTree(Tree {
            inner_language: None,
        });
        let nodes = Value::Nodes(vec![3, 99, 7]);
        let expected = Error::new(ErrorKind::Navigation, "foreign handle");
        assert_eq!(nodes.to_string(&nav).unwrap_err(), expected);
        assert_eq!(nodes.to_number(&nav).unwrap_err(), expected);
        let ctx = FunctionContext {
            navigator: &nav,
            node: &3,
            initial_node: &0,
            position: 1,
            size: 1,
        };
        for name in ["name", "local-name", "namespace-uri"] {
            assert_eq!(
                call(name, std::slice::from_ref(&nodes), &ctx, &mut 100_000)
                    .unwrap()
                    .unwrap_err(),
                expected
            );
        }
        assert_eq!(
            Value::Nodes(vec![99]).to_string(&nav).unwrap_err(),
            expected
        );
    }

    #[test]
    fn value_conversions_select_earliest_document_order_and_propagate_errors() {
        let nav = Tree {
            inner_language: None,
        };
        assert_eq!(Value::Nodes(vec![7, 3]).to_string(&nav).unwrap(), "3");
        assert_eq!(Value::Nodes(vec![7, 3]).to_number(&nav).unwrap(), 3.0);
        assert_eq!(Value::<u8>::Nodes(vec![]).to_string(&nav).unwrap(), "");
        assert!(Value::<u8>::Nodes(vec![]).to_number(&nav).unwrap().is_nan());
        assert_eq!(Value::Boolean(true).to_string(&nav).unwrap(), "true");
        assert_eq!(Value::Boolean(false).to_number(&nav).unwrap(), 0.0);
        assert_eq!(
            Value::Nodes(vec![99]).to_string(&nav).unwrap_err().kind,
            ErrorKind::Navigation
        );
        assert_eq!(
            Value::<u8>::Number(1.0).as_nodes().unwrap_err().kind,
            ErrorKind::Type
        );
    }
    #[test]
    fn context_position_size_count_and_default_conversions() {
        assert_eq!(run("position", &[]).unwrap(), n(2.0));
        assert_eq!(run("last", &[]).unwrap(), n(5.0));
        assert_eq!(run("count", &[Value::Nodes(vec![3, 7])]).unwrap(), n(2.0));
        assert_eq!(run("string", &[]).unwrap(), s("3"));
        assert_eq!(run("number", &[]).unwrap(), n(3.0));
        assert_eq!(run("number", &[Value::Boolean(true)]).unwrap(), n(1.0));
        assert_eq!(run("string-length", &[]).unwrap(), n(1.0));
        assert_eq!(run("normalize-space", &[]).unwrap(), s("3"));
    }
    #[test]
    fn names_use_earliest_node_and_correct_node_kind_semantics() {
        assert_eq!(
            run("name", &[Value::Nodes(vec![7, 1])]).unwrap(),
            s("p:outer")
        );
        assert_eq!(
            run("local-name", &[Value::Nodes(vec![7, 1])]).unwrap(),
            s("outer")
        );
        assert_eq!(
            run("namespace-uri", &[Value::Nodes(vec![7, 1])]).unwrap(),
            s("urn:test")
        );
        for func in ["name", "local-name", "namespace-uri"] {
            assert_eq!(run(func, &[Value::Nodes(vec![])]).unwrap(), s(""));
            assert_eq!(run(func, &[]).unwrap(), s(""));
        }
        assert_eq!(run("name", &[Value::Nodes(vec![6])]).unwrap(), s("target"));
        assert_eq!(run("name", &[Value::Nodes(vec![5])]).unwrap(), s("p"));
        assert_eq!(
            run("namespace-uri", &[Value::Nodes(vec![5])]).unwrap(),
            s("")
        );
    }
    #[test]
    fn string_functions_convert_arguments_and_handle_empty_needles() {
        assert_eq!(
            run("concat", &[s("a"), n(2.0), Value::Boolean(true)]).unwrap(),
            s("a2true")
        );
        assert_eq!(
            run("starts-with", &[s("é🙂"), s("é")]).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            run("contains", &[s("é🙂"), s("🙂")]).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            run("substring-before", &[s("ab:cd:ef"), s(":")]).unwrap(),
            s("ab")
        );
        assert_eq!(
            run("substring-after", &[s("ab:cd:ef"), s(":")]).unwrap(),
            s("cd:ef")
        );
        for func in ["starts-with", "contains"] {
            assert_eq!(run(func, &[s(""), s("")]).unwrap(), Value::Boolean(true));
        }
        assert_eq!(run("substring-before", &[s("abc"), s("")]).unwrap(), s(""));
        assert_eq!(
            run("substring-after", &[s("abc"), s("")]).unwrap(),
            s("abc")
        );
        for func in ["substring-before", "substring-after"] {
            assert_eq!(run(func, &[s("abc"), s("z")]).unwrap(), s(""));
        }
    }
    #[test]
    fn unicode_strings_count_scalar_characters_and_translate_first_occurrence() {
        assert_eq!(run("string-length", &[s("é🙂e\u{301}")]).unwrap(), n(4.0));
        assert_eq!(
            run("substring", &[s("é🙂e\u{301}"), n(2.0), n(2.0)]).unwrap(),
            s("🙂e")
        );
        assert_eq!(
            run("translate", &[s("aba🙂é"), s("ab🙂aé"), s("XY💡Z")]).unwrap(),
            s("XYX💡")
        );
        assert_eq!(
            run("translate", &[s("abc"), s(""), s("XY")]).unwrap(),
            s("abc")
        );
    }
    #[test]
    fn substring_uses_xpath_rounded_position_inequalities() {
        for (start, len, expected) in [
            (1.5, 2.6, "234"),
            (0.0, 3.0, "12"),
            (f64::NAN, 3.0, ""),
            (1.0, f64::NAN, ""),
            (-42.0, f64::INFINITY, "12345"),
            (f64::NEG_INFINITY, f64::INFINITY, ""),
            (2.0, -1.0, ""),
        ] {
            assert_eq!(
                run("substring", &[s("12345"), n(start), n(len)]).unwrap(),
                s(expected)
            );
        }
        assert_eq!(run("substring", &[s("12345"), n(3.0)]).unwrap(), s("345"));
        assert_eq!(
            run("substring", &[s("12345"), n(f64::NEG_INFINITY)]).unwrap(),
            s("12345")
        );
        assert_eq!(run("substring", &[s("12345"), n(f64::NAN)]).unwrap(), s(""));
    }
    #[test]
    fn normalize_space_uses_xml_whitespace_only() {
        assert_eq!(
            run("normalize-space", &[s(" \t a\r\nb  c \n")]).unwrap(),
            s("a b c")
        );
        assert_eq!(
            run("normalize-space", &[s(" \u{a0}\u{b}\u{2003} ")]).unwrap(),
            s("\u{a0}\u{b}\u{2003}")
        );
    }
    #[test]
    fn boolean_functions_and_numeric_functions() {
        assert_eq!(
            run("boolean", &[n(f64::NAN)]).unwrap(),
            Value::Boolean(false)
        );
        assert_eq!(run("not", &[s("false")]).unwrap(), Value::Boolean(false));
        assert_eq!(run("true", &[]).unwrap(), Value::Boolean(true));
        assert_eq!(run("false", &[]).unwrap(), Value::Boolean(false));
        assert_eq!(run("sum", &[Value::Nodes(vec![7, 3])]).unwrap(), n(10.0));
        assert_eq!(run("sum", &[Value::Nodes(vec![])]).unwrap(), n(0.0));
        assert_eq!(run("floor", &[n(-1.2)]).unwrap(), n(-2.0));
        assert_eq!(run("ceiling", &[n(-1.2)]).unwrap(), n(-1.0));
        assert_eq!(run("round", &[n(-1.5)]).unwrap(), n(-1.0));
        let Value::Number(z) = run("round", &[n(-0.5)]).unwrap() else {
            panic!()
        };
        assert!(z.is_sign_negative());
    }
    #[test]
    fn id_tokenizes_every_nodeset_string_and_deduplicates_document_order() {
        assert_eq!(
            run("id", &[s("b\ta\rc\nb a missing")]).unwrap(),
            Value::Nodes(vec![1, 3, 7])
        );
        assert_eq!(
            run("id", &[Value::Nodes(vec![9])]).unwrap(),
            Value::Nodes(vec![1, 7])
        );
        assert_eq!(run("id", &[s("a\u{a0}b")]).unwrap(), Value::Nodes(vec![]));
        assert_eq!(
            run("id", &[Value::Nodes(vec![])]).unwrap(),
            Value::Nodes(vec![])
        );
    }
    #[test]
    fn lang_inherits_case_insensitively_and_closest_declaration_shadows() {
        for language in ["en", "EN", "en-us"] {
            assert_eq!(run("lang", &[s(language)]).unwrap(), Value::Boolean(true));
        }
        for language in ["e", "en-u", "fr", ""] {
            assert_eq!(run("lang", &[s(language)]).unwrap(), Value::Boolean(false));
        }
        let nav = Tree {
            inner_language: Some("fr"),
        };
        assert_eq!(
            run_at("lang", &[s("en")], &nav, 8).unwrap(),
            Value::Boolean(false)
        );
        assert_eq!(
            run_at("lang", &[s("FR")], &nav, 4).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            run_at("lang", &[s("en")], &nav, 0).unwrap(),
            Value::Boolean(false)
        );
    }
    #[test]
    fn exact_arity_type_errors_and_unknown_extension_dispatch() {
        let arities = [
            ("last", 0, 0),
            ("position", 0, 0),
            ("count", 1, 1),
            ("id", 1, 1),
            ("local-name", 0, 1),
            ("namespace-uri", 0, 1),
            ("name", 0, 1),
            ("string", 0, 1),
            ("concat", 2, usize::MAX),
            ("starts-with", 2, 2),
            ("contains", 2, 2),
            ("substring-before", 2, 2),
            ("substring-after", 2, 2),
            ("substring", 2, 3),
            ("string-length", 0, 1),
            ("normalize-space", 0, 1),
            ("translate", 3, 3),
            ("boolean", 1, 1),
            ("not", 1, 1),
            ("true", 0, 0),
            ("false", 0, 0),
            ("lang", 1, 1),
            ("number", 0, 1),
            ("sum", 1, 1),
            ("floor", 1, 1),
            ("ceiling", 1, 1),
            ("round", 1, 1),
        ];
        for (name, min, max) in arities {
            if min > 0 {
                assert_eq!(
                    run(name, &vec![s(""); min - 1]).unwrap_err().kind,
                    ErrorKind::Arity,
                    "{name}"
                );
            }
            if max < usize::MAX {
                assert_eq!(
                    run(name, &vec![s(""); max + 1]).unwrap_err().kind,
                    ErrorKind::Arity,
                    "{name}"
                );
            }
        }
        for name in ["count", "sum", "local-name", "namespace-uri", "name"] {
            assert_eq!(run(name, &[n(1.0)]).unwrap_err().kind, ErrorKind::Type);
        }
        let nav = Tree {
            inner_language: None,
        };
        let ctx = FunctionContext {
            navigator: &nav,
            node: &8,
            initial_node: &0,
            position: 1,
            size: 1,
        };
        for name in [
            "evaluate",
            "document",
            "upper-case",
            "lower-case",
            "ends-with",
            "p:string",
        ] {
            assert!(call(name, &[], &ctx, &mut 100_000).is_none(), "{name}");
        }
    }
    struct MeteredChain {
        parents: std::cell::Cell<usize>,
        attribute_names: std::cell::Cell<usize>,
        strings: std::cell::Cell<usize>,
        ids: std::cell::Cell<usize>,
        with_attributes: bool,
    }
    impl MeteredChain {
        fn new(with_attributes: bool) -> Self {
            Self {
                parents: 0.into(),
                attribute_names: 0.into(),
                strings: 0.into(),
                ids: 0.into(),
                with_attributes,
            }
        }
    }
    impl Navigator for MeteredChain {
        type Node = u32;
        fn kind(&self, node: &u32) -> NodeKind {
            match *node {
                0 => NodeKind::Root,
                u32::MAX => NodeKind::Attribute,
                _ => NodeKind::Element,
            }
        }
        fn parent(&self, node: &u32) -> Result<Option<u32>> {
            self.parents.set(self.parents.get() + 1);
            Ok(node.checked_sub(1))
        }
        fn children<'a>(&'a self, node: &'a u32) -> impl Iterator<Item = Result<u32>> + 'a {
            (*node < 10_000).then(|| Ok(node + 1)).into_iter()
        }
        fn attributes<'a>(&'a self, _: &'a u32) -> impl Iterator<Item = Result<u32>> + 'a {
            self.with_attributes.then_some(Ok(u32::MAX)).into_iter()
        }
        fn name(&self, _: &u32) -> Result<Option<ExpandedName>> {
            self.attribute_names.set(self.attribute_names.get() + 1);
            Ok(None)
        }
        fn string_value(&self, _: &u32) -> Result<String> {
            self.strings.set(self.strings.get() + 1);
            Ok("3".into())
        }
        fn element_by_id(&self, _: &u32, _: &str) -> Result<Option<u32>> {
            self.ids.set(self.ids.get() + 1);
            Ok(Some(1))
        }
        fn compare_document_order(&self, a: &u32, b: &u32) -> Ordering {
            a.cmp(b)
        }
    }
    fn metered(
        name: &str,
        args: &[Value<u32>],
        nav: &MeteredChain,
        budget: &mut usize,
    ) -> Result<Value<u32>> {
        let ctx = FunctionContext {
            navigator: nav,
            node: &10_000,
            initial_node: &0,
            position: 1,
            size: 1,
        };
        call(name, args, &ctx, budget).unwrap()
    }
    #[test]
    fn lang_navigation_stops_before_exceeding_parent_and_attribute_budgets() {
        let nav = MeteredChain::new(false);
        let mut budget = 2;
        assert_eq!(
            metered("lang", &[Value::String("en".into())], &nav, &mut budget)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.parents.get(), 2);
        assert_eq!(budget, 0);
        let nav = MeteredChain::new(false);
        assert_eq!(
            metered("lang", &[Value::String("en".into())], &nav, &mut 0)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.parents.get(), 0);
        let nav = MeteredChain::new(true);
        assert_eq!(
            metered("lang", &[Value::String("en".into())], &nav, &mut 1)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.attribute_names.get(), 1);
        assert_eq!(nav.parents.get(), 0);
        let nav = MeteredChain::new(true);
        assert_eq!(
            metered("lang", &[Value::String("en".into())], &nav, &mut 0)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.attribute_names.get(), 0);
    }
    #[test]
    fn id_and_sum_charge_each_node_read_and_id_lookup() {
        let nav = MeteredChain::new(false);
        for name in ["id", "sum"] {
            assert_eq!(
                metered(name, &[Value::Nodes(vec![3])], &nav, &mut 0)
                    .unwrap_err()
                    .kind,
                ErrorKind::Limit
            );
        }
        assert_eq!(nav.strings.get(), 0);
        assert_eq!(
            metered("id", &[Value::String("a".into())], &nav, &mut 0)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.ids.get(), 0);
        assert_eq!(
            metered("id", &[Value::Nodes(vec![3])], &nav, &mut 1)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.strings.get(), 1);
        assert_eq!(nav.ids.get(), 0);
        let mut budget = 2;
        assert_eq!(
            metered("id", &[Value::Nodes(vec![3])], &nav, &mut budget).unwrap(),
            Value::Nodes(vec![1])
        );
        assert_eq!(budget, 0);
        let nav = MeteredChain::new(false);
        assert_eq!(
            metered("sum", &[Value::Nodes(vec![3, 4])], &nav, &mut 1)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(nav.strings.get(), 1);
        assert_eq!(
            metered("sum", &[Value::Nodes(vec![])], &nav, &mut 0).unwrap(),
            Value::Number(0.0)
        );
        assert_eq!(
            metered("id", &[Value::Nodes(vec![])], &nav, &mut 0).unwrap(),
            Value::Nodes(vec![])
        );
    }

    #[test]
    fn navigation_errors_are_preserved() {
        for name in [
            "string",
            "number",
            "sum",
            "normalize-space",
            "string-length",
        ] {
            assert_eq!(
                run(name, &[Value::Nodes(vec![99])]).unwrap_err().kind,
                ErrorKind::Navigation
            );
        }
        assert_eq!(
            run("id", &[s("fail")]).unwrap_err().kind,
            ErrorKind::Navigation
        );
    }
}
