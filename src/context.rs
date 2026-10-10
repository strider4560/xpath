use crate::{Error, ErrorKind, ExpandedName, Navigator, Result, Value};
use std::{collections::BTreeMap, rc::Rc};

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

type Function<'a, N> = Rc<
    dyn for<'b> Fn(
            &FunctionContext<'b, N>,
            &[Value<<N as Navigator>::Node>],
        ) -> Result<Value<<N as Navigator>::Node>>
        + 'a,
>;

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
    pub(crate) variables: BTreeMap<ExpandedName, Value<N::Node>>,
    pub(crate) functions: BTreeMap<ExpandedName, Function<'a, N>>,
    pub(crate) limits: Limits,
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
            variables: BTreeMap::new(),
            functions: BTreeMap::new(),
            limits: Limits::default(),
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
