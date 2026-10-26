//! R10: cooperative cancellation and deadlines.
use std::{
    cell::Cell,
    cmp::Ordering,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
    time::{Duration, Instant},
};
use xpath1::{Context, ErrorKind, ExpandedName, Limits, Navigator, NodeKind, Result, Value, XPath};

/// A complete tree of fanout 10 and depth 4 (11,110 elements under the
/// root, numbered in preorder). It counts every navigator call, and can set
/// a flag or sleep once `trigger` child nodes have been read.
struct Wide<'f> {
    nodes: Vec<(Option<usize>, Vec<usize>)>,
    calls: Cell<usize>,
    children_read: Cell<usize>,
    trigger: usize,
    flag: Option<&'f AtomicBool>,
    sleep: Option<Duration>,
    /// Children read when the trigger fired.
    fired_at: Cell<Option<usize>>,
}

impl<'f> Wide<'f> {
    fn new() -> Self {
        let mut nodes = vec![(None, Vec::new())];
        fn grow(nodes: &mut Vec<(Option<usize>, Vec<usize>)>, parent: usize, depth: usize) {
            if depth == 4 {
                return;
            }
            for _ in 0..10 {
                let id = nodes.len();
                nodes.push((Some(parent), Vec::new()));
                nodes[parent].1.push(id);
                grow(nodes, id, depth + 1);
            }
        }
        grow(&mut nodes, 0, 0);
        Self {
            nodes,
            calls: Cell::new(0),
            children_read: Cell::new(0),
            trigger: usize::MAX,
            flag: None,
            sleep: None,
            fired_at: Cell::new(None),
        }
    }
    fn call(&self) {
        self.calls.set(self.calls.get() + 1);
    }
    fn read_child(&self) {
        let read = self.children_read.get() + 1;
        self.children_read.set(read);
        if read == self.trigger {
            self.fired_at.set(Some(read));
            if let Some(flag) = self.flag {
                flag.store(true, AtomicOrdering::Relaxed);
            }
            if let Some(sleep) = self.sleep {
                std::thread::sleep(sleep);
            }
        }
    }
    /// Child nodes read after the trigger fired.
    fn read_after_trigger(&self) -> usize {
        self.children_read.get() - self.fired_at.get().expect("trigger fired")
    }
}

impl Navigator for Wide<'_> {
    type Node = usize;
    fn validate_node(&self, _: &usize) -> Result<()> {
        self.call();
        Ok(())
    }
    fn kind(&self, node: &usize) -> NodeKind {
        self.call();
        if *node == 0 {
            NodeKind::Root
        } else {
            NodeKind::Element
        }
    }
    fn parent(&self, node: &usize) -> Result<Option<usize>> {
        self.call();
        Ok(self.nodes[*node].0)
    }
    fn children<'a>(&'a self, node: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        self.call();
        self.nodes[*node].1.iter().map(move |&child| {
            self.read_child();
            Ok(child)
        })
    }
    fn name(&self, node: &usize) -> Result<Option<ExpandedName>> {
        self.call();
        Ok((*node != 0).then(|| ExpandedName::local("n")))
    }
    fn string_value(&self, _: &usize) -> Result<String> {
        self.call();
        Ok(String::new())
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}

const ALL: &str = "count(//node())";

fn run(nav: &Wide<'_>, ctx: Context<'_, Wide<'_>>) -> xpath1::Result<Value<usize>> {
    let _ = nav;
    XPath::compile(ALL).unwrap().evaluate(&ctx)
}

#[test]
fn without_cancellation_results_are_unchanged() {
    let flag = AtomicBool::new(false);
    let nav = Wide::new();
    let far = Instant::now() + Duration::from_secs(3600);
    let plain = run(&nav, Context::new(&nav, 0)).unwrap();
    assert_eq!(plain, Value::Number(11_110.0));
    let limits = Limits {
        deadline: Some(far),
        ..Limits::default()
    };
    let checked = run(
        &nav,
        Context::new(&nav, 0).with_cancel(&flag).with_limits(limits),
    )
    .unwrap();
    assert_eq!(checked, plain);
}

#[test]
fn a_set_flag_ends_evaluation_before_any_navigation() {
    let flag = AtomicBool::new(true);
    let nav = Wide::new();
    let error = run(&nav, Context::new(&nav, 0).with_cancel(&flag)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert!(error.message.contains("cancelled"), "{error}");
    assert_eq!(nav.calls.get(), 0);
    // Clearing the flag lets the same context evaluate again.
    flag.store(false, AtomicOrdering::Relaxed);
    let ctx = Context::new(&nav, 0).with_cancel(&flag);
    assert_eq!(run(&nav, ctx).unwrap(), Value::Number(11_110.0));
}

#[test]
fn a_past_deadline_ends_evaluation_before_any_navigation() {
    let nav = Wide::new();
    let limits = Limits {
        deadline: Some(Instant::now()),
        ..Limits::default()
    };
    let error = run(&nav, Context::new(&nav, 0).with_limits(limits)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert!(error.message.contains("deadline"), "{error}");
    assert_eq!(nav.calls.get(), 0);
}

#[test]
fn a_flag_set_mid_evaluation_ends_it_within_the_cadence() {
    // Each child node read costs at least one work unit, so after the flag is
    // set at most one check interval of reads can follow.
    for (max_operations, interval) in [
        (1_000_000, 3906),
        (100_000, 390),
        (25_600, 100),
        (2_000_000_000, 4096),
        (100, 1),
    ] {
        let flag = AtomicBool::new(false);
        let mut nav = Wide::new();
        nav.flag = Some(&flag);
        nav.trigger = 50;
        let limits = Limits {
            max_operations,
            ..Limits::default()
        };
        let ctx = Context::new(&nav, 0).with_cancel(&flag).with_limits(limits);
        let error = run(&nav, ctx).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit, "{max_operations}");
        if max_operations > 100 {
            assert!(error.message.contains("cancelled"), "{error}");
            assert!(
                nav.read_after_trigger() <= interval,
                "{max_operations}: {} reads after the flag",
                nav.read_after_trigger()
            );
        }
    }
}

#[test]
fn a_deadline_passing_mid_evaluation_ends_it() {
    let mut nav = Wide::new();
    nav.trigger = 50;
    nav.sleep = Some(Duration::from_millis(30));
    let limits = Limits {
        deadline: Some(Instant::now() + Duration::from_millis(15)),
        max_operations: 100_000,
        ..Limits::default()
    };
    let error = run(&nav, Context::new(&nav, 0).with_limits(limits)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert!(error.message.contains("deadline"), "{error}");
    assert!(nav.read_after_trigger() <= 390);
}

#[test]
fn the_flag_can_be_set_from_another_thread() {
    let flag = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            let nav = Wide::new();
            let ctx = Context::new(&nav, 0).with_cancel(&flag);
            let expression = XPath::compile(ALL).unwrap();
            // Evaluate until the other thread cancels.
            loop {
                if let Err(error) = expression.evaluate(&ctx) {
                    return error;
                }
            }
        });
        std::thread::sleep(Duration::from_millis(20));
        flag.store(true, AtomicOrdering::Relaxed);
        assert_eq!(handle.join().unwrap().kind, ErrorKind::Limit);
    });
}
