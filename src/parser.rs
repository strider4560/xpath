use crate::{ast::*, Axis, Error, ErrorKind, Result};
use std::ops::Range;

// Bounds apply to compilation independently of the evaluator's per-context limits.
// AST height also bounds recursive evaluation, cloning, and destruction.
const MAX_SOURCE_BYTES: usize = 1_048_576;
const MAX_TOKENS: usize = 65_536;
const MAX_NESTING: usize = 64;
const MAX_AST_HEIGHT: usize = 128;

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Name(QName),
    NamespaceWildcard(String),
    String(String),
    Number(f64),
    Slash,
    DoubleSlash,
    Dot,
    DoubleDot,
    At,
    DoubleColon,
    Dollar,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Comma,
    Plus,
    Minus,
    Star,
    Pipe,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    End,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    span: Range<usize>,
}

fn limit(message: &str) -> Error {
    Error::new(ErrorKind::Limit, message)
}

/// XML 1.0 Fifth Edition NCNameStartChar (NameStartChar without colon).
fn name_start(c: char) -> bool {
    matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{c0}'..='\u{d6}' |
        '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' |
        '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' |
        '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' |
        '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}
fn name_char(c: char) -> bool {
    name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' |
        '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}
fn ncname_end(source: &str, start: usize) -> usize {
    let mut end = start;
    for c in source[start..].chars() {
        if !name_char(c) {
            break;
        }
        end += c.len_utf8();
    }
    end
}

fn lex(source: &str) -> Result<Vec<Token>> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(limit("XPath source exceeds compilation byte limit"));
    }
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if matches!(bytes[at], b' ' | b'\t' | b'\r' | b'\n') {
            at += 1;
            continue;
        }
        if tokens.len() >= MAX_TOKENS {
            return Err(limit("XPath exceeds compilation token limit"));
        }
        let start = at;
        let c = source[at..].chars().next().unwrap();
        let kind = if name_start(c) {
            at = ncname_end(source, at);
            let first = source[start..at].to_owned();
            if bytes.get(at) == Some(&b':') && bytes.get(at + 1) != Some(&b':') {
                at += 1;
                if bytes.get(at) == Some(&b'*') {
                    at += 1;
                    Kind::NamespaceWildcard(first)
                } else {
                    let local_start = at;
                    if !source[at..].chars().next().is_some_and(name_start) {
                        return Err(Error::syntax(
                            "expected NCName after namespace prefix",
                            at..at,
                        ));
                    }
                    at = ncname_end(source, at);
                    Kind::Name(QName {
                        prefix: Some(first),
                        local: source[local_start..at].to_owned(),
                    })
                }
            } else {
                Kind::Name(QName {
                    prefix: None,
                    local: first,
                })
            }
        } else if c.is_ascii_digit()
            || (c == '.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            // XPath numbers have no sign or exponent in their lexical production.
            Kind::Number(
                source[start..at]
                    .parse()
                    .map_err(|_| Error::syntax("invalid number", start..at))?,
            )
        } else if c == '\'' || c == '"' {
            at += 1;
            let content = at;
            match source[at..].find(c) {
                Some(end) => {
                    at += end;
                    let value = source[content..at].to_owned();
                    at += 1;
                    Kind::String(value)
                }
                None => {
                    return Err(Error::syntax(
                        "unterminated string literal",
                        start..source.len(),
                    ))
                }
            }
        } else {
            at += c.len_utf8();
            match c {
                '/' if bytes.get(at) == Some(&b'/') => {
                    at += 1;
                    Kind::DoubleSlash
                }
                '/' => Kind::Slash,
                '.' if bytes.get(at) == Some(&b'.') => {
                    at += 1;
                    Kind::DoubleDot
                }
                '.' => Kind::Dot,
                ':' if bytes.get(at) == Some(&b':') => {
                    at += 1;
                    Kind::DoubleColon
                }
                '@' => Kind::At,
                '$' => Kind::Dollar,
                '(' => Kind::LeftParen,
                ')' => Kind::RightParen,
                '[' => Kind::LeftBracket,
                ']' => Kind::RightBracket,
                ',' => Kind::Comma,
                '+' => Kind::Plus,
                '-' => Kind::Minus,
                '*' => Kind::Star,
                '|' => Kind::Pipe,
                '=' => Kind::Eq,
                '!' if bytes.get(at) == Some(&b'=') => {
                    at += 1;
                    Kind::Ne
                }
                '<' if bytes.get(at) == Some(&b'=') => {
                    at += 1;
                    Kind::Le
                }
                '>' if bytes.get(at) == Some(&b'=') => {
                    at += 1;
                    Kind::Ge
                }
                '<' => Kind::Lt,
                '>' => Kind::Gt,
                _ => return Err(Error::syntax("unexpected character", start..at)),
            }
        };
        tokens.push(Token {
            kind,
            span: start..at,
        });
    }
    tokens.push(Token {
        kind: Kind::End,
        span: at..at,
    });
    Ok(tokens)
}

struct Parsed {
    expr: Expr,
    height: usize,
}
impl Parsed {
    fn new(expr: Expr, height: usize) -> Result<Self> {
        if height > MAX_AST_HEIGHT {
            return Err(limit("XPath exceeds compilation AST height limit"));
        }
        Ok(Self { expr, height })
    }
    fn leaf(expr: Expr) -> Self {
        Self { expr, height: 1 }
    }
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    nesting: usize,
}

pub(crate) fn parse(source: &str) -> Result<Expr> {
    let mut parser = Parser {
        tokens: lex(source)?,
        at: 0,
        nesting: 0,
    };
    let expression = parser.expression()?;
    if !matches!(parser.peek(), Kind::End) {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(expression.expr)
}

impl Parser {
    fn peek(&self) -> &Kind {
        &self.tokens[self.at].kind
    }
    fn peek_next(&self) -> &Kind {
        &self.tokens[(self.at + 1).min(self.tokens.len() - 1)].kind
    }
    fn take(&mut self) -> Kind {
        let kind = self.tokens[self.at].kind.clone();
        if !matches!(kind, Kind::End) {
            self.at += 1;
        }
        kind
    }
    fn eat(&mut self, kind: Kind) -> bool {
        if self.peek() == &kind {
            self.take();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, kind: Kind, message: &str) -> Result<()> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    fn error(&self, message: &str) -> Error {
        Error::syntax(message, self.tokens[self.at].span.clone())
    }
    fn keyword(&self, name: &str) -> bool {
        matches!(self.peek(), Kind::Name(QName { prefix: None, local }) if local == name)
    }
    fn expression(&mut self) -> Result<Parsed> {
        if self.nesting >= MAX_NESTING {
            return Err(limit("XPath exceeds compilation nesting limit"));
        }
        self.nesting += 1;
        let result = self.or();
        self.nesting -= 1;
        result
    }
    fn binary(op: BinaryOp, lhs: Parsed, rhs: Parsed) -> Result<Parsed> {
        let height = lhs.height.max(rhs.height) + 1;
        Parsed::new(
            Expr::Binary(op, Box::new(lhs.expr), Box::new(rhs.expr)),
            height,
        )
    }
    fn or(&mut self) -> Result<Parsed> {
        let mut lhs = self.and()?;
        while self.keyword("or") {
            self.take();
            lhs = Self::binary(BinaryOp::Or, lhs, self.and()?)?;
        }
        Ok(lhs)
    }
    fn and(&mut self) -> Result<Parsed> {
        let mut lhs = self.equality()?;
        while self.keyword("and") {
            self.take();
            lhs = Self::binary(BinaryOp::And, lhs, self.equality()?)?;
        }
        Ok(lhs)
    }
    fn equality(&mut self) -> Result<Parsed> {
        let mut lhs = self.relational()?;
        loop {
            let op = match self.peek() {
                Kind::Eq => BinaryOp::Eq,
                Kind::Ne => BinaryOp::Ne,
                _ => break,
            };
            self.take();
            lhs = Self::binary(op, lhs, self.relational()?)?;
        }
        Ok(lhs)
    }
    fn relational(&mut self) -> Result<Parsed> {
        let mut lhs = self.additive()?;
        loop {
            let op = match self.peek() {
                Kind::Lt => BinaryOp::Lt,
                Kind::Le => BinaryOp::Le,
                Kind::Gt => BinaryOp::Gt,
                Kind::Ge => BinaryOp::Ge,
                _ => break,
            };
            self.take();
            lhs = Self::binary(op, lhs, self.additive()?)?;
        }
        Ok(lhs)
    }
    fn additive(&mut self) -> Result<Parsed> {
        let mut lhs = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Kind::Plus => BinaryOp::Add,
                Kind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.take();
            lhs = Self::binary(op, lhs, self.multiplicative()?)?;
        }
        Ok(lhs)
    }
    fn multiplicative(&mut self) -> Result<Parsed> {
        let mut lhs = self.unary()?;
        loop {
            let op = if self.eat(Kind::Star) {
                BinaryOp::Mul
            } else if self.keyword("div") {
                self.take();
                BinaryOp::Div
            } else if self.keyword("mod") {
                self.take();
                BinaryOp::Mod
            } else {
                break;
            };
            lhs = Self::binary(op, lhs, self.unary()?)?;
        }
        Ok(lhs)
    }
    fn unary(&mut self) -> Result<Parsed> {
        let mut minuses = 0;
        while self.eat(Kind::Minus) {
            minuses += 1;
            if minuses >= MAX_AST_HEIGHT {
                return Err(limit("XPath exceeds compilation AST height limit"));
            }
        }
        let mut value = self.union()?;
        for _ in 0..minuses {
            value = Parsed::new(Expr::Unary(Box::new(value.expr)), value.height + 1)?;
        }
        Ok(value)
    }
    fn union(&mut self) -> Result<Parsed> {
        let mut lhs = self.path()?;
        while self.eat(Kind::Pipe) {
            lhs = Self::binary(BinaryOp::Union, lhs, self.path()?)?;
        }
        Ok(lhs)
    }
    fn step_start(&self) -> bool {
        match self.peek() {
            Kind::Dot | Kind::DoubleDot | Kind::At | Kind::Star | Kind::NamespaceWildcard(_) => {
                true
            }
            Kind::Name(name) => !matches!(self.peek_next(), Kind::LeftParen) || node_type(name),
            _ => false,
        }
    }
    fn path(&mut self) -> Result<Parsed> {
        if self.eat(Kind::Slash) {
            let mut steps = Vec::new();
            let mut height = 1;
            if self.step_start() {
                self.relative_steps(&mut steps, &mut height)?;
            }
            return Parsed::new(
                Expr::Path {
                    base: None,
                    absolute: true,
                    steps,
                },
                height,
            );
        }
        if self.eat(Kind::DoubleSlash) {
            let mut steps = vec![descendant_or_self()];
            let mut height = 1;
            self.relative_steps(&mut steps, &mut height)?;
            return Parsed::new(
                Expr::Path {
                    base: None,
                    absolute: true,
                    steps,
                },
                height,
            );
        }
        if self.step_start() {
            let mut steps = Vec::new();
            let mut height = 1;
            self.relative_steps(&mut steps, &mut height)?;
            return Parsed::new(
                Expr::Path {
                    base: None,
                    absolute: false,
                    steps,
                },
                height,
            );
        }
        let mut base = self.primary()?;
        let (predicates, predicate_height) = self.predicates()?;
        if !predicates.is_empty() {
            let height = base.height.max(predicate_height) + 1;
            base = Parsed::new(
                Expr::Filter {
                    base: Box::new(base.expr),
                    predicates,
                },
                height,
            )?;
        }
        if matches!(self.peek(), Kind::Slash | Kind::DoubleSlash) {
            let separator = self.take();
            let mut steps = Vec::new();
            if separator == Kind::DoubleSlash {
                steps.push(descendant_or_self());
            }
            let mut height = base.height + 1;
            self.relative_steps(&mut steps, &mut height)?;
            return Parsed::new(
                Expr::Path {
                    base: Some(Box::new(base.expr)),
                    absolute: false,
                    steps,
                },
                height,
            );
        }
        Ok(base)
    }
    fn relative_steps(&mut self, steps: &mut Vec<Step>, height: &mut usize) -> Result<()> {
        loop {
            let (step, predicate_height) = self.step()?;
            *height = (*height).max(predicate_height + 1);
            if *height > MAX_AST_HEIGHT {
                return Err(limit("XPath exceeds compilation AST height limit"));
            }
            steps.push(step);
            match self.peek() {
                Kind::Slash => {
                    self.take();
                }
                Kind::DoubleSlash => {
                    self.take();
                    steps.push(descendant_or_self());
                }
                _ => break,
            }
        }
        Ok(())
    }
    fn step(&mut self) -> Result<(Step, usize)> {
        if self.eat(Kind::Dot) {
            return Ok((
                Step {
                    axis: Axis::SelfAxis,
                    test: NodeTest::Node,
                    predicates: vec![],
                },
                0,
            ));
        }
        if self.eat(Kind::DoubleDot) {
            return Ok((
                Step {
                    axis: Axis::Parent,
                    test: NodeTest::Node,
                    predicates: vec![],
                },
                0,
            ));
        }
        let axis = if self.eat(Kind::At) {
            Axis::Attribute
        } else if matches!(self.peek_next(), Kind::DoubleColon) {
            let axis = match self.peek() {
                Kind::Name(QName {
                    prefix: None,
                    local,
                }) => match local.as_str() {
                    "child" => Axis::Child,
                    "descendant" => Axis::Descendant,
                    "parent" => Axis::Parent,
                    "ancestor" => Axis::Ancestor,
                    "following-sibling" => Axis::FollowingSibling,
                    "preceding-sibling" => Axis::PrecedingSibling,
                    "following" => Axis::Following,
                    "preceding" => Axis::Preceding,
                    "attribute" => Axis::Attribute,
                    "namespace" => Axis::Namespace,
                    "self" => Axis::SelfAxis,
                    "descendant-or-self" => Axis::DescendantOrSelf,
                    "ancestor-or-self" => Axis::AncestorOrSelf,
                    _ => return Err(self.error("unknown axis")),
                },
                _ => return Err(self.error("expected axis name")),
            };
            self.take();
            self.take();
            axis
        } else {
            Axis::Child
        };
        let test = match self.peek().clone() {
            Kind::Star => {
                self.take();
                NodeTest::Any
            }
            Kind::NamespaceWildcard(prefix) => {
                self.take();
                NodeTest::NamespaceWildcard(prefix)
            }
            Kind::Name(name) => {
                self.take();
                if node_type(&name) && self.eat(Kind::LeftParen) {
                    let test = match name.local.as_str() {
                        "node" => NodeTest::Node,
                        "text" => NodeTest::Text,
                        "comment" => NodeTest::Comment,
                        "processing-instruction" => {
                            let target = if let Kind::String(s) = self.peek() {
                                let s = s.clone();
                                self.take();
                                Some(s)
                            } else {
                                None
                            };
                            NodeTest::ProcessingInstruction(target)
                        }
                        _ => unreachable!(),
                    };
                    self.expect(Kind::RightParen, "expected ')' after node test")?;
                    test
                } else {
                    NodeTest::Name(name)
                }
            }
            _ => return Err(self.error("expected location step node test")),
        };
        let (predicates, height) = self.predicates()?;
        Ok((
            Step {
                axis,
                test,
                predicates,
            },
            height,
        ))
    }
    fn predicates(&mut self) -> Result<(Vec<Expr>, usize)> {
        let mut expressions = Vec::new();
        let mut height = 0;
        while self.eat(Kind::LeftBracket) {
            let predicate = self.expression()?;
            height = height.max(predicate.height);
            self.expect(Kind::RightBracket, "expected ']' after predicate")?;
            expressions.push(predicate.expr);
        }
        Ok((expressions, height))
    }
    fn primary(&mut self) -> Result<Parsed> {
        match self.peek().clone() {
            Kind::String(value) => {
                self.take();
                Ok(Parsed::leaf(Expr::String(value)))
            }
            Kind::Number(value) => {
                self.take();
                Ok(Parsed::leaf(Expr::Number(value)))
            }
            Kind::Dollar => {
                self.take();
                match self.peek().clone() {
                    Kind::Name(name) => {
                        self.take();
                        Ok(Parsed::leaf(Expr::Variable(name)))
                    }
                    _ => Err(self.error("expected variable QName")),
                }
            }
            Kind::LeftParen => {
                self.take();
                let expr = self.expression()?;
                self.expect(Kind::RightParen, "expected ')' after expression")?;
                Ok(expr)
            }
            Kind::Name(name) if !node_type(&name) => {
                self.take();
                self.expect(Kind::LeftParen, "expected '(' after function name")?;
                let mut args = Vec::new();
                let mut height = 1;
                if !self.eat(Kind::RightParen) {
                    loop {
                        let arg = self.expression()?;
                        height = height.max(arg.height + 1);
                        args.push(arg.expr);
                        if !self.eat(Kind::Comma) {
                            break;
                        }
                    }
                    self.expect(Kind::RightParen, "expected ')' after function arguments")?;
                }
                Parsed::new(Expr::Function(name, args), height)
            }
            _ => Err(self.error("expected XPath expression")),
        }
    }
}

fn node_type(name: &QName) -> bool {
    name.prefix.is_none()
        && matches!(
            name.local.as_str(),
            "node" | "text" | "comment" | "processing-instruction"
        )
}
fn descendant_or_self() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: NodeTest::Node,
        predicates: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, ErrorKind};

    fn path(s: &str) -> (Option<Box<Expr>>, bool, Vec<Step>) {
        match parse(s).unwrap() {
            Expr::Path {
                base,
                absolute,
                steps,
            } => (base, absolute, steps),
            e => panic!("expected path: {e:?}"),
        }
    }

    #[test]
    fn all_axes_and_node_tests() {
        for (name, axis) in [
            ("child", Axis::Child),
            ("descendant", Axis::Descendant),
            ("parent", Axis::Parent),
            ("ancestor", Axis::Ancestor),
            ("following-sibling", Axis::FollowingSibling),
            ("preceding-sibling", Axis::PrecedingSibling),
            ("following", Axis::Following),
            ("preceding", Axis::Preceding),
            ("attribute", Axis::Attribute),
            ("namespace", Axis::Namespace),
            ("self", Axis::SelfAxis),
            ("descendant-or-self", Axis::DescendantOrSelf),
            ("ancestor-or-self", Axis::AncestorOrSelf),
        ] {
            let (_, _, steps) = path(&format!("{name}::*"));
            assert_eq!(steps[0].axis, axis);
            assert_eq!(steps[0].test, NodeTest::Any);
        }
        for (source, test) in [
            ("node()", NodeTest::Node),
            ("text()", NodeTest::Text),
            ("comment()", NodeTest::Comment),
            (
                "processing-instruction()",
                NodeTest::ProcessingInstruction(None),
            ),
            (
                "processing-instruction('xml')",
                NodeTest::ProcessingInstruction(Some("xml".into())),
            ),
            ("p:*", NodeTest::NamespaceWildcard("p".into())),
        ] {
            assert_eq!(path(source).2[0].test, test);
        }
    }

    #[test]
    fn abbreviated_paths_and_filters() {
        let (_, absolute, steps) = path("//a/@b/../.");
        assert!(absolute);
        assert_eq!(
            steps.iter().map(|s| s.axis).collect::<Vec<_>>(),
            vec![
                Axis::DescendantOrSelf,
                Axis::Child,
                Axis::Attribute,
                Axis::Parent,
                Axis::SelfAxis
            ]
        );
        assert!(path("/").1);
        let (base, absolute, steps) = path("(//a)[1][2]/b");
        assert!(!absolute);
        assert_eq!(steps.len(), 1);
        assert!(matches!(*base.unwrap(), Expr::Filter { predicates, .. } if predicates.len() == 2));
        assert_eq!(path("a[b][c]").2[0].predicates.len(), 2);
        assert!(matches!(path("$x//y").0, Some(b) if matches!(*b, Expr::Variable(_))));
    }

    #[test]
    fn operator_precedence_and_associativity() {
        let e = parse("1 or 2 and 3 = 4 < 5 + 6 * -7 | /a").unwrap();
        assert!(
            matches!(e, Expr::Binary(BinaryOp::Or, _, b) if matches!(*b, Expr::Binary(BinaryOp::And, _, _)))
        );
        assert!(
            matches!(parse("1 - 2 - 3").unwrap(), Expr::Binary(BinaryOp::Sub, a, _) if matches!(*a, Expr::Binary(BinaryOp::Sub, _, _)))
        );
        assert!(
            matches!(parse("-a | b").unwrap(), Expr::Unary(a) if matches!(*a, Expr::Binary(BinaryOp::Union, _, _)))
        );
        assert!(matches!(
            parse("a * b").unwrap(),
            Expr::Binary(BinaryOp::Mul, _, _)
        ));
        for keyword in ["or", "and", "div", "mod"] {
            assert_eq!(path(keyword).2.len(), 1);
            assert!(matches!(
                parse(&format!("a {keyword} b")).unwrap(),
                Expr::Binary(_, _, _)
            ));
            assert!(parse(&format!("/{keyword}/{keyword}")).is_ok());
        }
    }

    #[test]
    fn unicode_names_variables_and_functions() {
        assert_eq!(path("é:名/\u{200c}node/a\u{b7}b").2.len(), 3);
        assert!(
            matches!(parse("$p:变量").unwrap(), Expr::Variable(QName { prefix: Some(p), local }) if p == "p" && local == "变量")
        );
        assert!(
            matches!(parse("p:f('héllo', .5, 1., 2)").unwrap(), Expr::Function(_, args) if args.len() == 4)
        );
        assert!(parse("𐀀").is_ok());
        assert!(parse("é - 1").is_ok());
    }

    #[test]
    fn rejects_invalid_grammar_and_lexemes() {
        for source in [
            "",
            " ",
            "/numbers numbers",
            "/a/b[c > d]efg",
            "/inv/child::",
            "/invoice/@test[abcd",
            "/invoice/@test[abcd > x",
            "string-length('a",
            "/descendant::()",
            "(1 + 1",
            "a/",
            "//",
            "a//",
            "foo::a",
            "@",
            "a[1,2]",
            "f(,1)",
            "f(1,)",
            "1e2",
            "0x10",
            "a : b",
            "a:",
            "a:b:c",
            "$p:*",
            "a!b",
            "a == b",
            "a && b",
            "text('x')",
            "node(1)",
            "processing-instruction(1)",
            "processing-instruction('a','b')",
            ".[1]",
            "..[1]",
            "a\u{a0}/b",
            "\u{301}x",
            "*:a",
            ":a",
        ] {
            assert_eq!(
                parse(source).unwrap_err().kind,
                ErrorKind::Syntax,
                "{source:?}"
            );
        }
    }

    #[test]
    fn syntax_spans_are_utf8_byte_offsets() {
        for (source, span) in [("é/", 3..3), ("é !", 3..4), ("f('é", 2..5), ("a[b", 3..3)] {
            assert_eq!(parse(source).unwrap_err().span, Some(span), "{source:?}");
        }
    }

    #[test]
    fn depth_and_work_limits_reject_without_overflow() {
        for source in [
            format!("{}1{}", "(".repeat(1000), ")".repeat(1000)),
            format!("{}1", "-".repeat(2000)),
            "1+".repeat(5000) + "1",
            "a/".repeat(70000) + "a",
            " ".repeat(2_000_000),
        ] {
            assert_eq!(parse(&source).unwrap_err().kind, ErrorKind::Limit);
        }
    }
}
