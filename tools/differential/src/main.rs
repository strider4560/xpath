use std::io::{self, BufRead};
use xpath1::{Context, XPath};

fn main() {
    let document = xpath1::xml::Document::parse(include_str!("../fixture.xml")).unwrap();
    for line in io::stdin().lock().lines() {
        let expression = line.unwrap();
        let result = XPath::compile(&expression).and_then(|xpath| {
            xpath
                .evaluate(&Context::new(&document, document.root()))
                .and_then(|value| value.to_string(&document))
        });
        match result {
            Ok(value) => println!("{value}"),
            Err(error) => println!("ERR:{error:?}"),
        }
    }
}
