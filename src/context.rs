use crate::{Error, ErrorKind, ExpandedName, Namespaces, Navigator, NodeSetPool, Result, Value};
use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

/// Work limits for the evaluator. Extension callbacks must enforce their own limits.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// AST evaluations plus node-set processing, visited axis nodes, parent
    /// links and core-function navigation. Does not meter host callback internals
    /// or individual bytes in string operations.
    pub max_operations: usize,
    /// Maximum nested evaluation depth.
    pub max_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 1_000_000,
            max_depth: 128,
        }
    }
}

/// The dynamic focus supplied to an application-defined function.
pub struct FunctionContext<'a, N: Navigator> {
    pub navigator: &'a N,
    pub node: &'a N::Node,
    /// The original context node; useful for YANG's current() extension.
    pub initial_node: &'a N::Node,
    pub position: usize,
    pub size: usize,
}

type DynFunction<'a, N> = dyn for<'b> Fn(
        &FunctionContext<'b, N>,
        &[Value<<N as Navigator>::Node>],
    ) -> Result<Value<<N as Navigator>::Node>>
    + 'a;
type Function<'a, N> = Rc<DynFunction<'a, N>>;

type SharedFunction<N> = Box<DynFunction<'static, N>>;

/// Extension functions registered once and shared by many contexts through
/// [`Context::with_functions`], instead of re-registering closures per context.
pub struct FunctionTable<N: Navigator> {
    functions: HashMap<ExpandedName, SharedFunction<N>>,
}
impl<N: Navigator> FunctionTable<N> {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }
    /// Register (or replace) the function `name`.
    pub fn insert<F>(&mut self, name: ExpandedName, function: F) -> &mut Self
    where
        F: for<'b> Fn(&FunctionContext<'b, N>, &[Value<N::Node>]) -> Result<Value<N::Node>>
            + 'static,
    {
        self.functions.insert(name, Box::new(function));
        self
    }
    pub(crate) fn get(&self, name: &ExpandedName) -> Option<&SharedFunction<N>> {
        self.functions.get(name)
    }
}
impl<N: Navigator> Default for FunctionTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Namespace, variable and function bindings for one evaluation.
///
/// Namespace bindings are for the expression, independent of prefixes in the data.
/// An unprefixed name matches the null namespace, as specified by XPath 1.0.
pub struct Context<'a, N: Navigator> {
    pub(crate) navigator: &'a N,
    pub(crate) node: N::Node,
    pub(crate) position: usize,
    pub(crate) size: usize,
    pub(crate) namespaces: BTreeMap<String, String>,
    pub(crate) default_element_namespace: String,
    pub(crate) shared_namespaces: Option<&'a Namespaces>,
    pub(crate) variables: BTreeMap<ExpandedName, Value<N::Node>>,
    pub(crate) functions: BTreeMap<ExpandedName, Function<'a, N>>,
    pub(crate) shared_functions: Option<&'a FunctionTable<N>>,
    pub(crate) limits: Limits,
    pub(crate) pool: Option<&'a NodeSetPool<N::Node>>,
}
impl<'a, N: Navigator> Context<'a, N> {
    pub fn new(navigator: &'a N, node: N::Node) -> Self {
        Self {
            navigator,
            node,
            position: 1,
            size: 1,
            namespaces: BTreeMap::new(),
            default_element_namespace: String::new(),
            shared_namespaces: None,
            variables: BTreeMap::new(),
            functions: BTreeMap::new(),
            shared_functions: None,
            limits: Limits::default(),
            pool: None,
        }
    }
    pub fn with_namespace(mut self, prefix: impl Into<String>, uri: impl Into<String>) -> Self {
        self.namespaces.insert(prefix.into(), uri.into());
        self
    }
    /// Opt into an embedding language's namespace for unprefixed element tests.
    ///
    /// Leave unset for standard XPath 1.0. YANG can set the defining module's
    /// namespace here. This never affects attributes, namespace nodes, variables,
    /// functions, or explicitly prefixed names.
    pub fn with_default_element_namespace(mut self, uri: impl Into<String>) -> Self {
        self.default_element_namespace = uri.into();
        self
    }
    /// Resolve names through a shared [`Namespaces`] set as well.
    ///
    /// The `xml` prefix is always bound to its fixed URI; other prefixes
    /// resolve through this context's own [`with_namespace`](Self::with_namespace)
    /// bindings first, then the shared set. The default element namespace is
    /// this context's own if set, else the shared set's. When the context has
    /// no bindings of its own, resolved names come from (and are added to) the
    /// set's cache, so warm evaluations resolve names without allocating.
    pub fn with_namespaces(mut self, namespaces: &'a Namespaces) -> Self {
        self.shared_namespaces = Some(namespaces);
        self
    }
    /// Take intermediate node-set buffers from `pool` and give them back
    /// after use, so warm evaluations reuse them instead of allocating.
    ///
    /// Results are unchanged. The node-set an evaluation returns is owned by
    /// the caller and never given back by the evaluator; vectors returned by
    /// [`Navigator::children_named`] are given to the pool after use.
    pub fn with_pool(mut self, pool: &'a NodeSetPool<N::Node>) -> Self {
        self.pool = Some(pool);
        self
    }
    pub fn with_variable(self, name: impl Into<String>, value: Value<N::Node>) -> Self {
        self.with_variable_ns(ExpandedName::local(name), value)
    }
    pub fn with_variable_ns(mut self, name: ExpandedName, value: Value<N::Node>) -> Self {
        self.variables.insert(name, value);
        self
    }
    pub fn with_function<F>(mut self, name: ExpandedName, function: F) -> Self
    where
        F: for<'b> Fn(&FunctionContext<'b, N>, &[Value<N::Node>]) -> Result<Value<N::Node>> + 'a,
    {
        self.functions.insert(name, Rc::new(function));
        self
    }
    /// Use a shared [`FunctionTable`]. Functions added with
    /// [`with_function`](Self::with_function) shadow table entries of the same
    /// name; core XPath functions shadow both.
    pub fn with_functions(mut self, table: &'a FunctionTable<N>) -> Self {
        self.shared_functions = Some(table);
        self
    }
    /// Re-focus this context on `node` at position 1 of 1, keeping its
    /// namespace, variable and function bindings and limits, so one context
    /// can serve many evaluations.
    pub fn set_node(&mut self, node: N::Node) {
        self.node = node;
        self.position = 1;
        self.size = 1;
    }
    pub(crate) fn function(&self, name: &ExpandedName) -> Option<&DynFunction<'a, N>> {
        if let Some(function) = self.functions.get(name) {
            return Some(&**function);
        }
        Some(&**self.shared_functions?.get(name)?)
    }
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
    pub fn with_focus(mut self, position: usize, size: usize) -> Result<Self> {
        if position == 0 || position > size {
            return Err(Error::new(
                ErrorKind::Type,
                "context position must be in 1..=size",
            ));
        }
        self.position = position;
        self.size = size;
        Ok(self)
    }
}
