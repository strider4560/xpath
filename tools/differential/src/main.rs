use std::io::{self, BufRead};
use xpath1::{conformance::ScanHooks, xml::Document, Context, Navigator, XPath};

/// Evaluate each expression read from stdin at the fixture's root.
///
/// With `XPATH1_HOOKS=1` the document is wrapped in `ScanHooks`, so every
/// storage hook answers; the output must be byte-identical to a plain run.
fn main() {
    let document = Document::parse(include_str!("../fixture.xml")).unwrap();
    let root = document.root();
    if std::env::var_os("XPATH1_HOOKS").is_some_and(|v| v == "1") {
        run(&ScanHooks::new(document), root);
    } else {
        run(&document, root);
    }
}

fn run<N: Navigator>(navigator: &N, root: N::Node) {
    for line in io::stdin().lock().lines() {
        let expression = line.unwrap();
        let result = XPath::compile(&expression).and_then(|xpath| {
            xpath
                .evaluate(&Context::new(navigator, root.clone()))
                .and_then(|value| value.to_string(navigator))
        });
        match result {
            Ok(value) => println!("{value}"),
            Err(error) => println!("ERR:{error:?}"),
        }
    }
}
