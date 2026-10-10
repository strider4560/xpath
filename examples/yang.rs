use xpath1::{xml::Document, Context, Error, ErrorKind, ExpandedName, Value, XPath};

fn main() -> xpath1::Result<()> {
    let document = Document::parse(
        "<interfaces xmlns='urn:example:interfaces'><interface><name>eth0</name><enabled>true</enabled></interface><selected>eth0</selected></interfaces>"
    )?;
    let root_context = Context::new(&document, document.root())
        .with_default_element_namespace("urn:example:interfaces");
    let selected = XPath::compile("/interfaces/selected")?.select(&root_context)?[0];
    let context = Context::new(&document, selected)
        .with_default_element_namespace("urn:example:interfaces")
        .with_function(ExpandedName::local("current"), |focus, args| {
            if !args.is_empty() {
                return Err(Error::new(ErrorKind::Arity, "current expects no arguments"));
            }
            Ok(Value::Nodes(vec![*focus.initial_node]))
        });
    let must = XPath::compile("../interface[name = current()]/enabled = 'true'")?;
    assert!(must.evaluate(&context)?.to_boolean());
    println!("selected interface is enabled");
    Ok(())
}
