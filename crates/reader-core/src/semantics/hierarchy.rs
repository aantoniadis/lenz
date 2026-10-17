//! A call hierarchy: a tree of the functions calling (or called by) one function, loaded a level at a time as
//! nodes unfold.

use serde_json::{Value, json};

use crate::search::Hit;

/// Which way the tree goes from its root: calls, or types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Callers,
    Callees,
    Supertypes,
    Subtypes,
}

impl Direction {
    /// A type hierarchy's direction (not a call hierarchy's).
    pub fn is_type(self) -> bool {
        matches!(self, Self::Supertypes | Self::Subtypes)
    }
    /// The other way, in the same kind of hierarchy.
    pub fn other(self) -> Direction {
        match self {
            Self::Callers => Self::Callees,
            Self::Callees => Self::Callers,
            Self::Supertypes => Self::Subtypes,
            Self::Subtypes => Self::Supertypes,
        }
    }
}

/// Where a hierarchy's levels come from: the server's own requests, or its usages read against syntax relations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Server,
    Syntax,
}

/// A function in a server's answer: its item (sent back to ask for its own level), name, symbol kind, how many
/// times it calls (or is called) there, and the call site as a location (`uri`, `range`).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub item: Value,
    pub name: String,
    pub kind: u64,
    /// Its kind read from syntax (a syntax node; `None`: the server's symbol kind).
    pub decl: Option<lenz_content::highlight::DeclKind>,
    pub calls: usize,
    pub at: Value,
}

impl Call {
    /// Its kind, for a row (`path`: its file, for the language's own word).
    pub fn label(&self, path: &str) -> &'static str {
        match (self.decl, self.kind) {
            (Some(d), _) => crate::semantics::kind_label(d, path),
            // A name read at the caret: its kind is not known.
            (None, 0) => "",
            (None, kind) => crate::semantics::kind_label(crate::semantics::decl_kind(kind), path),
        }
    }
}

/// What is known of a node's level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Children {
    NotAsked,
    /// Asked in this session request.
    Loading(u64),
    /// Loaded; `more` were beyond the limit.
    Loaded { more: usize },
    Failed(String),
}

/// A function in the tree, at its call site.
#[derive(Clone, Debug)]
pub struct Node {
    pub call: Call,
    pub site: Hit,
    pub depth: u16,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub state: Children,
    pub open: bool,
    /// It is one of its own ancestors: shown, never unfolded.
    pub recursive: bool,
}

/// The tree: nodes in the order they arrived, the root first.
#[derive(Clone, Debug)]
pub struct Hierarchy {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    /// The server session its items belong to.
    pub session: u64,
    pub source: Source,
}

/// Which function an item is: its file and where its name starts.
fn identity(item: &Value) -> (Value, Value) {
    (item["uri"].clone(), item["selectionRange"]["start"].clone())
}

impl Hierarchy {
    /// A tree of `root` (at `site`), unfolded but not yet asked about.
    pub fn new(root: Call, site: Hit, direction: Direction, session: u64, source: Source) -> Self {
        let node = Node { call: root, site, depth: 0, parent: None, children: Vec::new(), state: Children::NotAsked, open: true, recursive: false };
        Self { direction, nodes: vec![node], session, source }
    }

    /// The nodes shown: depth first, children only of unfolded nodes.
    pub fn visible(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            out.push(i);
            let n = &self.nodes[i];
            if n.open && !n.recursive {
                stack.extend(n.children.iter().rev());
            }
        }
        out
    }

    /// The node whose level request `request` is loading.
    pub fn loading(&self, request: u64) -> Option<usize> {
        self.nodes.iter().position(|n| n.state == Children::Loading(request))
    }

    /// `node`'s level arrived for `request` (else it is stale and dropped): at most `limit` children, each marked
    /// when it is one of its ancestors.
    pub fn loaded(&mut self, node: usize, request: u64, level: Vec<(Call, Hit)>, limit: usize) {
        if self.nodes.get(node).is_none_or(|n| n.state != Children::Loading(request)) {
            return;
        }
        let mut ancestors = Vec::new();
        let mut at = Some(node);
        while let Some(i) = at {
            ancestors.push(identity(&self.nodes[i].call.item));
            at = self.nodes[i].parent;
        }
        let more = level.len().saturating_sub(limit);
        let depth = self.nodes[node].depth + 1;
        for (call, site) in level.into_iter().take(limit) {
            let recursive = ancestors.contains(&identity(&call.item));
            self.nodes.push(Node { call, site, depth, parent: Some(node), children: Vec::new(), state: Children::NotAsked, open: false, recursive });
            let id = self.nodes.len() - 1;
            self.nodes[node].children.push(id);
        }
        self.nodes[node].state = Children::Loaded { more };
        self.nodes[node].open = true;
    }

    /// `node`'s level for `request` failed (unfolding it again asks again).
    pub fn failed(&mut self, node: usize, request: u64, why: String) {
        if let Some(n) = self.nodes.get_mut(node).filter(|n| n.state == Children::Loading(request)) {
            n.state = Children::Failed(why);
        }
    }
}

/// The root of a `prepareCallHierarchy` answer (its first item), at its name.
pub fn prepare(reply: &Value) -> Option<Call> {
    let item = reply.as_array()?.first()?;
    Some(Call { item: item.clone(), name: item["name"].as_str().unwrap_or_default().to_owned(), kind: item["kind"].as_u64().unwrap_or(0), decl: None, calls: 1, at: json!({ "uri": item["uri"], "range": item["selectionRange"] }) })
}

/// An `incomingCalls` / `outgoingCalls` answer about a function in `parent_uri`: each caller at its first call
/// site (in the caller's file), or each callee where the parent calls it (in the parent's file).
pub fn calls(reply: &Value, direction: Direction, parent_uri: &str) -> Vec<Call> {
    let (key, site_uri) = match direction {
        Direction::Callers => ("from", None),
        Direction::Callees => ("to", Some(parent_uri)),
        Direction::Supertypes | Direction::Subtypes => return Vec::new(), // a type level: [`types`]
    };
    let Some(items) = reply.as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|c| {
            let item = c.get(key)?;
            let ranges = c["fromRanges"].as_array().map_or(&[][..], Vec::as_slice);
            let range = ranges.first().unwrap_or(&item["selectionRange"]);
            let uri = site_uri.map_or_else(|| item["uri"].clone(), |u| json!(u));
            Some(Call { item: item.clone(), name: item["name"].as_str().unwrap_or_default().to_owned(), kind: item["kind"].as_u64().unwrap_or(0), decl: None, calls: ranges.len().max(1), at: json!({ "uri": uri, "range": range }) })
        })
        .collect()
}

/// A `typeHierarchy/supertypes` or `/subtypes` answer: each type at its name.
pub fn types(reply: &Value) -> Vec<Call> {
    let Some(items) = reply.as_array() else { return Vec::new() };
    items
        .iter()
        .map(|item| Call { item: item.clone(), name: item["name"].as_str().unwrap_or_default().to_owned(), kind: item["kind"].as_u64().unwrap_or(0), decl: None, calls: 1, at: json!({ "uri": item["uri"], "range": item["selectionRange"] }) })
        .collect()
}

/// `level` with one entry per place (the first kept).
pub fn merge(level: Vec<(Call, Hit)>) -> Vec<(Call, Hit)> {
    let mut seen = std::collections::HashSet::new();
    level.into_iter().filter(|(c, _)| seen.insert(identity(&c.item))).collect()
}

/// [`merge`] for a level whose places are located targets.
pub fn merge_targets(level: Vec<(Call, crate::semantics::Target)>) -> Vec<(Call, crate::semantics::Target)> {
    let mut seen = std::collections::HashSet::new();
    level.into_iter().filter(|(c, _)| seen.insert(identity(&c.item))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, line: u64) -> Call {
        let range = json!({ "start": { "line": line, "character": 3 }, "end": { "line": line, "character": 4 } });
        Call { item: json!({ "name": name, "kind": 12, "uri": "file:///a.rs", "range": range, "selectionRange": range }), name: name.into(), kind: 12, decl: None, calls: 1, at: json!({ "uri": "file:///a.rs", "range": range }) }
    }

    fn site(line: usize) -> crate::search::Hit {
        crate::search::Hit { path: "/a.rs".into(), line, range: 3..4, text: "".into(), decl: None, origin: None }
    }

    fn node(name: &str, line: u64) -> (Call, crate::search::Hit) {
        (call(name, line), site(line as usize))
    }

    #[test]
    fn levels_load_lazily_and_a_cycle_is_marked_not_followed() {
        let (root, at) = node("a", 0);
        let mut h = Hierarchy::new(root, at, Direction::Callers, 1, Source::Server);
        assert_eq!(h.visible(), [0]);
        h.nodes[0].state = Children::Loading(7);
        h.loaded(0, 7, vec![node("b", 5), node("a", 0)], 500);
        assert_eq!(h.visible(), [0, 1, 2], "the root opens with its level");
        assert!(h.nodes[2].recursive, "a calls itself: marked");
        assert!(!h.nodes[1].recursive);
        assert_eq!((h.nodes[1].depth, h.nodes[1].parent), (1, Some(0)));
        h.loaded(0, 8, vec![node("zzz", 9)], 500); // a stale request
        assert_eq!(h.nodes[0].children.len(), 2);
        h.nodes[1].state = Children::Loading(9);
        assert_eq!(h.loading(9), Some(1));
        h.failed(1, 9, "boom".into());
        assert_eq!(h.nodes[1].state, Children::Failed("boom".into()));
        // b's callers: a (an ancestor: marked) and five more, of which the limit keeps four.
        h.nodes[1].state = Children::Loading(10);
        let mut level = vec![node("a", 0)];
        level.extend((0..5).map(|i| node(&format!("c{i}"), 20 + i)));
        h.loaded(1, 10, level, 5);
        assert_eq!(h.nodes[1].children.len(), 5);
        assert_eq!(h.nodes[1].state, Children::Loaded { more: 1 });
        assert!(h.nodes[h.nodes[1].children[0]].recursive, "a called through b: still a cycle");
        // Folding hides a level without forgetting it.
        h.nodes[1].open = false;
        assert_eq!(h.visible(), [0, 1, 2]);
    }

    #[test]
    fn replies_become_calls_with_their_call_sites() {
        let item = |name: &str, line: u64| json!({ "name": name, "kind": 6, "uri": format!("file:///{name}.rs"), "range": { "start": { "line": line, "character": 0 }, "end": { "line": line + 2, "character": 1 } }, "selectionRange": { "start": { "line": line, "character": 3 }, "end": { "line": line, "character": 7 } } });
        let r = |line: u64| json!({ "start": { "line": line, "character": 4 }, "end": { "line": line, "character": 8 } });
        let parent = "file:///self.rs";
        let incoming = json!([{ "from": item("main", 0), "fromRanges": [r(1), r(2)] }]);
        let callers = calls(&incoming, Direction::Callers, parent);
        assert_eq!((callers[0].name.as_str(), callers[0].kind, callers[0].calls), ("main", 6, 2));
        assert_eq!(callers[0].at, json!({ "uri": "file:///main.rs", "range": r(1) }), "a caller's site is in the caller's file");
        let outgoing = json!([{ "to": item("helper", 9), "fromRanges": [r(3)] }]);
        assert_eq!(calls(&outgoing, Direction::Callees, parent)[0].at, json!({ "uri": parent, "range": r(3) }), "a callee's site is where it is called from");
        let root = prepare(&json!([item("helper", 9)])).unwrap();
        assert_eq!(root.at["range"], item("helper", 9)["selectionRange"]);
        assert!(prepare(&json!(null)).is_none() && prepare(&json!([])).is_none());
    }

    #[test]
    fn type_items_become_nodes_at_their_names() {
        let reply = json!([{ "name": "Circle", "kind": 5, "uri": "file:///c.java", "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }, "selectionRange": { "start": { "line": 2, "character": 6 }, "end": { "line": 2, "character": 12 } } }]);
        let t = types(&reply);
        assert_eq!((t[0].name.as_str(), t[0].kind, t[0].label("/c.java")), ("Circle", 5, "type"));
        assert_eq!(t[0].at, json!({ "uri": "file:///c.java", "range": reply[0]["selectionRange"] }));
        assert!(types(&json!(null)).is_empty());
    }

    #[test]
    fn merge_keeps_one_row_per_place() {
        let (a, b) = (node("A", 3), node("B", 5));
        let merged = merge(vec![a.clone(), b, a]);
        assert_eq!(merged.iter().map(|(c, _)| c.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    }

    #[test]
    fn directions_pair_up() {
        assert_eq!((Direction::Subtypes.other(), Direction::Callers.other()), (Direction::Supertypes, Direction::Callees));
        assert!(Direction::Supertypes.is_type() && !Direction::Callees.is_type());
        let syntax = Call { decl: Some(lenz_content::highlight::DeclKind::Impl), ..call("S", 1) };
        assert_eq!(syntax.label("/a.rs"), "impl");
    }

    #[test]
    fn a_rust_interface_kind_is_a_trait() {
        let syntax = Call { decl: Some(lenz_content::highlight::DeclKind::Interface), ..call("Show", 1) };
        assert_eq!((syntax.label("/src/lib.rs"), syntax.label("/src/Shape.java")), ("trait", "interface"));
        let server = Call { kind: 11, ..call("Show", 1) };
        assert_eq!((server.label("/src/lib.rs"), server.label("/a.ts")), ("trait", "interface"));
    }
}
