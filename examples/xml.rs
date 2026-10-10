use xpath1::{xml::Document, Context, XPath};

fn main() -> xpath1::Result<()> {
    let document = Document::parse("<interfaces><interface enabled='true'>eth0</interface><interface enabled='false'>eth1</interface></interfaces>")?;
    let xpath = XPath::compile("/interfaces/interface[@enabled = 'true']")?;
    let context = Context::new(&document, document.root());
    println!("{}", xpath.evaluate(&context)?.to_string(&document)?);
    Ok(())
}
