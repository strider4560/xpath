use crate::{Error, ErrorKind, ExpandedName, Namespaces, Navigator, NodeSetPool, Result, Value};
use std::{
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
    rc::Rc,
    sync::atomic::AtomicBool,
    time::Instant,
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
    /// Wall-clock deadline. Checked when an evaluation starts and then once
    /// per `max_operations / 256` work units (at least every 4096, and at
    /// most every unit); once it has passed, evaluation ends with
    /// [`ErrorKind::Limit`]. A cancellation flag set with
    /// [`Context::with_cancel`] is checked on the same cadence.
    pub deadline: Option<Instant>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 1_000_000,
            max_depth: 128,
            deadline: None,
        }
    }
}
impl Limits {
    /// Work units between two deadline / cancellation checks.
    pub(crate) fn check_interval(&self) -> usize {
        (self.max_operations / 256).clamp(1, 4096)
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

/// An extension function stored in a [`FunctionTable`] built with
/// [`FunctionTable::new`]: any `'static` closure.
pub type LocalFunction<N> = DynFunction<'static, N>;

/// An extension function stored in a [`FunctionTable`] built with
/// [`FunctionTable::new_sync`]: a `'static` closure that is `Send + Sync`.
pub type SyncFunction<N> = dyn for<'b> Fn(
        &FunctionContext<'b, N>,
        &[Value<<N as Navigator>::Node>],
    ) -> Result<Value<<N as Navigator>::Node>>
    + Send
    + Sync
    + 'static;

mod sealed {
    pub trait Sealed {}
}

/// The function types a [`FunctionTable`] can hold: [`LocalFunction`] and
/// [`SyncFunction`]. Sealed.
pub trait TableFunction<N: Navigator>: sealed::Sealed {
    #[doc(hidden)]
    fn as_local(&self) -> &LocalFunction<N>;
}
impl<N: Navigator> sealed::Sealed for LocalFunction<N> {}
impl<N: Navigator> TableFunction<N> for LocalFunction<N> {
    fn as_local(&self) -> &LocalFunction<N> {
        self
    }
}
impl<N: Navigator> sealed::Sealed for SyncFunction<N> {}
impl<N: Navigator> TableFunction<N> for SyncFunction<N> {
    fn as_local(&self) -> &LocalFunction<N> {
        self
    }
}

/// Extension functions registered once and shared by many contexts through
/// [`Context::with_functions`], instead of re-registering closures per context.
///
/// The second parameter is the stored closure type, and the table is
/// `Send + Sync` exactly when its closures are: [`FunctionTable::new`] accepts
/// any `'static` closure and builds a per-thread table;
/// [`FunctionTable::new_sync`] accepts only `Send + Sync` closures and builds
/// a [`SyncFunctionTable`] that can be shared across threads (for example in
/// an `Arc` or a `static`), whatever the navigator type.
///
/// ```compile_fail
/// # use xpath1::{FunctionTable, Navigator};
/// fn shared<T: Send + Sync>(_: &T) {}
/// // A `new` table may hold closures that are not Send or Sync.
/// fn check<N: Navigator>(table: &FunctionTable<N>) {
///     shared(table)
/// }
/// ```
///
/// ```
/// # use xpath1::{ExpandedName, FunctionTable, Navigator, SyncFunctionTable, Value};
/// fn shared<T: Send + Sync>(_: &T) {}
/// // Whatever the navigator, even one whose handles are not Send.
/// fn check<N: Navigator>(table: &SyncFunctionTable<N>) {
///     shared(table)
/// }
/// fn build<N: Navigator>() -> SyncFunctionTable<N> {
///     let mut table = FunctionTable::new_sync();
///     table.insert(ExpandedName::local("one"), |_, _| Ok(Value::Number(1.0)));
///     table
/// }
/// ```
pub struct FunctionTable<N: Navigator, F: ?Sized + TableFunction<N> = LocalFunction<N>> {
    functions: HashMap<ExpandedName, Box<F>>,
    navigator: PhantomData<fn(&N)>,
}

/// A [`FunctionTable`] of `Send + Sync` closures, itself `Send + Sync`.
pub type SyncFunctionTable<N> = FunctionTable<N, SyncFunction<N>>;

impl<N: Navigator> FunctionTable<N> {
    /// An empty per-thread table, accepting any `'static` closure.
    pub fn new() -> Self {
        Self::empty()
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
}
impl<N: Navigator> FunctionTable<N, SyncFunction<N>> {
    /// An empty table that accepts only `Send + Sync` closures and so is
    /// itself `Send + Sync`.
    pub fn new_sync() -> Self {
        Self::empty()
    }
    /// Register (or replace) the function `name`.
    pub fn insert<F>(&mut self, name: ExpandedName, function: F) -> &mut Self
    where
        F: for<'b> Fn(&FunctionContext<'b, N>, &[Value<N::Node>]) -> Result<Value<N::Node>>
            + Send
            + Sync
            + 'static,
    {
        self.functions.insert(name, Box::new(function));
        self
    }
}
impl<N: Navigator, F: ?Sized + TableFunction<N>> FunctionTable<N, F> {
    fn empty() -> Self {
        Self {
            functions: HashMap::new(),
            navigator: PhantomData,
        }
    }
}
impl<N: Navigator, F: ?Sized + TableFunction<N>> Default for FunctionTable<N, F> {
    fn default() -> Self {
        Self::empty()
    }
}

/// Lookup in a table of either closure type, erased for [`Context`].
trait Functions<N: Navigator> {
    fn get(&self, name: &ExpandedName) -> Option<&LocalFunction<N>>;
}
impl<N: Navigator, F: ?Sized + TableFunction<N>> Functions<N> for FunctionTable<N, F> {
    fn get(&self, name: &ExpandedName) -> Option<&LocalFunction<N>> {
        self.functions.get(name).map(|function| function.as_local())
    }
}

/// Namespace, variable and function bindings for one evaluation.
///
/// A context is a per-thread object: build one per caller (or re-focus one
/// with [`set_node`](Self::set_node)) rather than sharing it across threads.
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
    shared_functions: Option<&'a dyn Functions<N>>,
    pub(crate) limits: Limits,
    pub(crate) pool: Option<&'a NodeSetPool<N::Node>>,
    pub(crate) cancel: Option<&'a AtomicBool>,
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
            cancel: None,
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
    ///
    /// Accepts either kind of table ([`FunctionTable::new`] or
    /// [`FunctionTable::new_sync`]).
    pub fn with_functions<F>(mut self, table: &'a FunctionTable<N, F>) -> Self
    where
        F: ?Sized + TableFunction<N>,
    {
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
        self.shared_functions?.get(name)
    }
    /// End evaluations with [`ErrorKind::Limit`] once `flag` is set (for
    /// example by another thread or an RPC-level timeout). The flag is read
    /// when an evaluation starts and then on the cadence described at
    /// [`Limits::deadline`]; the evaluator never writes it.
    pub fn with_cancel(mut self, flag: &'a AtomicBool) -> Self {
        self.cancel = Some(flag);
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
