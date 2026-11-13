//! Effective namespace and reference resolution for Format 2025.10.
//!
//! Source nodes are borrowed, never rewritten. Views identify placements in the
//! expanded namespace; their ordered source layers retain duplicate members and
//! declaration paths. Only group envelopes are overlaid, never token values.
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::Diagnostic;
use crate::json::Value;
use crate::structure::{SUPPORTED_TYPES, child_path};

type NodeId = usize;
pub(crate) type TokenId = usize;
type ViewId = usize;
type Fallible<T> = Result<T, ()>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Group,
    Token,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Slot {
    Typed(&'static str),
    Any,
    Literal,
    Dimensions,
    ShadowItem,
    GradientStop,
}

impl Slot {
    fn expected(self) -> Expected<'static> {
        match self {
            Self::Typed(name) => Expected::Type(name),
            Self::ShadowItem => Expected::Type("shadow"),
            Self::Any | Self::GradientStop => Expected::Any,
            Self::Dimensions | Self::Literal => Expected::Literal,
        }
    }
}

/// Curly references are contextual; pointer wrappers are recognized in every
/// value slot. Metadata stays literal even when selected by a pointer.
#[derive(Clone, Copy)]
pub(crate) enum Expected<'a> {
    Type(&'a str),
    Any,
    Literal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Location {
    Namespace(ViewId),
    Raw(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ValueCursor {
    location: Location,
    owner: Option<ViewId>,
    opaque: bool,
    slot: Slot,
    selected_token: Option<TokenId>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedValue {
    pub(crate) cursor: ValueCursor,
    /// A complete token value was selected, not merely a compatible component
    /// or metadata member. This is the type source for whole-token aliases.
    pub(crate) alias: Option<TokenId>,
    /// Only curly aliases require token-type identity. JSON Pointers select
    /// compatible values, even when the source token has a different type.
    pub(crate) curly_alias: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeOrigin {
    Explicit,
    Referenced(TokenId),
    Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TokenType {
    pub(crate) name: &'static str,
    pub(crate) origin: TypeOrigin,
}

struct Node<'a> {
    value: &'a Value,
    path: String,
    children: Vec<NodeId>,
}

struct View {
    definitions: Vec<NodeId>,
    parent: Option<ViewId>,
    path: String,
    kind: Option<Fallible<Kind>>,
    layers: Option<Fallible<Rc<[NodeId]>>>,
    members: Option<Fallible<Rc<[(String, ViewId)]>>>,
    member_cache: HashMap<String, Fallible<Rc<[ViewId]>>>,
    value: Option<Fallible<ResolvedValue>>,
    token_type: Option<Fallible<TokenType>>,
}

#[derive(Clone, PartialEq, Eq)]
enum Operation {
    Kind(ViewId),
    Layers(ViewId),
    Members(ViewId),
    Member(ViewId, String),
    Value(ViewId),
    Type(ViewId),
    Head(ValueCursor, bool),
    #[cfg(test)]
    Scan(ValueCursor, Slot),
}

struct Frame {
    operation: Operation,
    owner: Option<ViewId>,
    site: String,
}

/// The core and the type validator share this resolver. `prepare` resolves the
/// group graph and enumerates surviving tokens. Value validation then uses
/// `token_type`, `token_value`, `resolve_head`, `value` and `child` without
/// materializing or flattening referenced arrays/objects.
pub(crate) struct Resolver<'a> {
    nodes: Vec<Node<'a>>,
    views: Vec<View>,
    placements: HashMap<(Option<ViewId>, Vec<NodeId>), ViewId>,
    original_views: Vec<ViewId>,
    tokens: Vec<TokenId>,
    stack: Vec<Frame>,
    group_edges: HashMap<NodeId, Vec<NodeId>>,
    cyclic_groups: HashSet<NodeId>,
    diagnostics: Vec<Diagnostic>,
    reported: HashSet<(String, String, String)>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(document: &'a Value) -> Self {
        let mut resolver = Self {
            nodes: Vec::new(),
            views: Vec::new(),
            placements: HashMap::new(),
            original_views: Vec::new(),
            tokens: Vec::new(),
            stack: Vec::new(),
            group_edges: HashMap::new(),
            cyclic_groups: HashSet::new(),
            diagnostics: Vec::new(),
            reported: HashSet::new(),
        };
        let root = resolver.index(document, String::new());
        resolver.original_view(root, None, "");
        resolver
    }

    fn index(&mut self, value: &'a Value, path: String) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            value,
            path,
            children: Vec::new(),
        });
        let children = match value {
            Value::Object(members) => members
                .iter()
                .map(|(key, value)| {
                    let path = child_path(&self.nodes[id].path, key);
                    self.index(value, path)
                })
                .collect(),
            Value::Array(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let path = child_path(&self.nodes[id].path, &index.to_string());
                    self.index(value, path)
                })
                .collect(),
            _ => Vec::new(),
        };
        self.nodes[id].children = children;
        id
    }

    fn original_view(&mut self, node: NodeId, parent: Option<ViewId>, name: &str) {
        let view = self.placement(vec![node], parent, name);
        self.original_views.push(view);
        if self.has(node, "$value") {
            return;
        }
        for (name, child) in self.namespace_children(node) {
            self.original_view(child, Some(view), &name);
        }
    }

    fn placement(
        &mut self,
        definitions: Vec<NodeId>,
        parent: Option<ViewId>,
        name: &str,
    ) -> ViewId {
        let key = (parent, definitions);
        if let Some(&view) = self.placements.get(&key) {
            return view;
        }
        let path = parent.map_or_else(String::new, |parent| {
            child_path(&self.views[parent].path, name)
        });
        let view = self.views.len();
        self.views.push(View {
            definitions: key.1.clone(),
            parent,
            path,
            kind: None,
            layers: None,
            members: None,
            member_cache: HashMap::new(),
            value: None,
            token_type: None,
        });
        self.placements.insert(key, view);
        view
    }

    fn namespace_children(&self, node: NodeId) -> Vec<(String, NodeId)> {
        let Value::Object(members) = self.nodes[node].value else {
            return Vec::new();
        };
        members
            .iter()
            .enumerate()
            .filter_map(|(index, (name, value))| {
                ((!name.starts_with('$') || name == "$root") && matches!(value, Value::Object(_)))
                    .then(|| (name.clone(), self.nodes[node].children[index]))
            })
            .collect()
    }

    fn has(&self, node: NodeId, key: &str) -> bool {
        matches!(self.nodes[node].value, Value::Object(members) if members.iter().any(|(name, _)| name == key))
    }

    // Structural metadata permits identical duplicates. Pointer lookup below
    // deliberately does not: RFC 6901 requires a unique object member.
    fn property(&self, node: NodeId, key: &str) -> Fallible<Option<NodeId>> {
        let Value::Object(members) = self.nodes[node].value else {
            return Ok(None);
        };
        let mut found: Option<NodeId> = None;
        for (index, (name, _)) in members.iter().enumerate() {
            if name == key {
                let candidate = self.nodes[node].children[index];
                if let Some(previous) = found {
                    if self.nodes[previous].value != self.nodes[candidate].value {
                        return Err(());
                    }
                } else {
                    found = Some(candidate);
                }
            }
        }
        Ok(found)
    }

    fn definition(&self, view: ViewId) -> NodeId {
        *self.views[view]
            .definitions
            .last()
            .expect("a view has a source definition")
    }

    fn reference_site(&self, view: ViewId) -> String {
        let node = self.definition(view);
        for key in ["$ref", "$extends", "$value"] {
            if self.has(node, key) {
                return child_path(&self.nodes[node].path, key);
            }
        }
        self.nodes[node].path.clone()
    }

    fn report(&mut self, path: String, code: &str, message: impl Into<String>) {
        let message = message.into();
        if self
            .reported
            .insert((path.clone(), code.to_owned(), message.clone()))
        {
            self.diagnostics.push(Diagnostic {
                path,
                code: code.to_owned(),
                message,
            });
        }
    }

    fn begin(&mut self, operation: Operation, owner: Option<ViewId>, site: String) -> Fallible<()> {
        if let Some(start) = self
            .stack
            .iter()
            .position(|frame| frame.operation == operation)
        {
            let mut participants: Vec<_> = self.stack[start..]
                .iter()
                .filter_map(|frame| frame.owner.map(|owner| (owner, frame.site.clone())))
                .collect();
            if let Some(owner) = owner {
                participants.push((owner, site));
            }
            // Every semantic participant is diagnosed, not just the back edge.
            let mut seen = HashSet::new();
            for (owner, site) in participants {
                if seen.insert(owner) {
                    self.report(site, "cycle", "Reference resolution contains a cycle.");
                }
            }
            return Err(());
        }
        self.stack.push(Frame {
            operation,
            owner,
            site,
        });
        Ok(())
    }

    fn end(&mut self) {
        self.stack.pop().expect("each successful begin has one end");
    }

    fn kind(&mut self, view: ViewId) -> Fallible<Kind> {
        if let Some(result) = self.views[view].kind {
            return result;
        }
        self.begin(Operation::Kind(view), Some(view), self.reference_site(view))?;
        let result = self.classify(view);
        self.end();
        self.views[view].kind = Some(result);
        result
    }

    fn classify(&mut self, view: ViewId) -> Fallible<Kind> {
        let node = self.definition(view);
        let value = self.property(node, "$value")?;
        let reference = self.property(node, "$ref")?;
        let extends = self.property(node, "$extends")?;
        if (reference.is_some() && (value.is_some() || extends.is_some()))
            || (value.is_some() && extends.is_some())
        {
            return Err(());
        }
        if value.is_some() {
            return Ok(Kind::Token);
        }
        let Some(reference) = reference else {
            return Ok(Kind::Group);
        };
        let children = self.namespace_children(node);
        if !children.is_empty() {
            return Ok(Kind::Group);
        }
        if self.has(node, "$type") && self.views[view].parent.is_some() {
            return Ok(Kind::Token);
        }
        let target = self.reference_pointer(reference)?;
        let kind = match target.location {
            Location::Namespace(target) => self.kind(target)?,
            Location::Raw(_) => Kind::Token,
        };
        if self.views[view].parent.is_none() && kind != Kind::Group {
            self.report(
                self.nodes[reference].path.clone(),
                "reference",
                "The document root must reference a group.",
            );
            return Err(());
        }
        Ok(kind)
    }

    fn group_layers(&mut self, view: ViewId) -> Fallible<Rc<[NodeId]>> {
        if let Some(result) = &self.views[view].layers {
            return result.clone();
        }
        self.begin(
            Operation::Layers(view),
            Some(view),
            self.reference_site(view),
        )?;
        let result = self.expand_layers(view);
        self.end();
        self.views[view].layers = Some(result.clone());
        result
    }

    fn expand_layers(&mut self, view: ViewId) -> Fallible<Rc<[NodeId]>> {
        if self.kind(view)? != Kind::Group {
            return Err(());
        }
        let definitions = self.views[view].definitions.clone();
        let mut layers = Vec::new();
        for node in definitions {
            let extension = self
                .property(node, "$extends")?
                .or(self.property(node, "$ref")?);
            if let Some(reference) = extension {
                let target = self.group_reference(reference, self.has(node, "$extends"))?;
                let targets = self.views[target].definitions.clone();
                self.group_edges.entry(node).or_default().extend(targets);
                layers.extend_from_slice(&self.group_layers(target)?);
            }
            layers.push(node);
        }
        Ok(layers.into())
    }

    fn group_reference(&mut self, reference: NodeId, curly_allowed: bool) -> Fallible<ViewId> {
        let text = match self.nodes[reference].value {
            Value::String(text) => text.as_str(),
            _ => {
                self.report(
                    self.nodes[reference].path.clone(),
                    "reference",
                    "A group reference must be a string.",
                );
                return Err(());
            }
        };
        let target = if curly_allowed && text.starts_with('{') {
            self.curly_target(text, reference)?
        } else {
            self.reference_pointer(reference)?
        };
        if let Location::Namespace(view) = target.location {
            if self.kind(view)? == Kind::Group {
                return Ok(view);
            }
        }
        self.report(
            self.nodes[reference].path.clone(),
            "reference",
            "A group extension must target a group.",
        );
        Err(())
    }

    fn members(&mut self, view: ViewId) -> Fallible<Rc<[(String, ViewId)]>> {
        if let Some(result) = &self.views[view].members {
            return result.clone();
        }
        self.begin(
            Operation::Members(view),
            Some(view),
            self.reference_site(view),
        )?;
        let result = self.enumerate_members(view);
        self.end();
        self.views[view].members = Some(result.clone());
        result
    }

    fn enumerate_members(&mut self, view: ViewId) -> Fallible<Rc<[(String, ViewId)]>> {
        let layers = self.group_layers(view)?;
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        for &layer in layers.iter() {
            for (name, _) in self.namespace_children(layer) {
                if seen.insert(name.clone()) {
                    names.push(name);
                }
            }
        }
        let mut members = Vec::new();
        for name in names {
            if let Ok(children) = self.member(view, &name) {
                for &child in children.iter() {
                    members.push((name.clone(), child));
                }
            }
        }
        Ok(members.into())
    }

    // Lookup forces one name, not every sibling. A local alias can therefore
    // classify itself by looking up an inherited or forward-declared sibling.
    fn member(&mut self, view: ViewId, name: &str) -> Fallible<Rc<[ViewId]>> {
        // Locating a declared child does not require expanding its parent.
        // In particular, a group may extend its own (non-cyclic) descendant.
        // Do not cache this pre-expansion lookup as the final merged member.
        if self
            .stack
            .iter()
            .any(|frame| frame.operation == Operation::Layers(view))
        {
            let definitions = self.views[view].definitions.clone();
            if definitions.iter().any(|&node| self.has(node, name)) {
                return self.merge_member_layers(view, name, &definitions);
            }
        }
        if let Some(result) = self.views[view].member_cache.get(name) {
            return result.clone();
        }
        self.begin(
            Operation::Member(view, name.to_owned()),
            Some(view),
            self.reference_site(view),
        )?;
        let result = self.merge_member(view, name);
        self.end();
        self.views[view]
            .member_cache
            .insert(name.to_owned(), result.clone());
        result
    }

    fn merge_member(&mut self, view: ViewId, name: &str) -> Fallible<Rc<[ViewId]>> {
        let layers = self.group_layers(view)?;
        self.merge_member_layers(view, name, &layers)
    }

    fn merge_member_layers(
        &mut self,
        view: ViewId,
        name: &str,
        layers: &[NodeId],
    ) -> Fallible<Rc<[ViewId]>> {
        let mut inherited = Vec::new();
        for &layer in layers.iter() {
            let children: Vec<_> = self
                .namespace_children(layer)
                .into_iter()
                .filter(|(key, _)| key == name)
                .map(|(_, node)| node)
                .collect();
            if children.is_empty() {
                continue;
            }
            let mut local: Vec<_> = children
                .iter()
                .map(|&child| self.placement(vec![child], Some(view), name))
                .collect();
            if local.len() == 1
                && inherited.len() == 1
                && self.kind(local[0])? == Kind::Group
                && self.kind(inherited[0])? == Kind::Group
            {
                let mut definitions = self.views[inherited[0]].definitions.clone();
                definitions.extend_from_slice(&children);
                local[0] = self.placement(definitions, Some(view), name);
            }
            inherited = local;
        }
        Ok(inherited.into())
    }

    fn namespace_cursor(&self, view: ViewId) -> ValueCursor {
        ValueCursor {
            location: Location::Namespace(view),
            owner: Some(view),
            opaque: false,
            slot: Slot::Any,
            selected_token: None,
        }
    }

    fn raw_cursor(
        &self,
        node: NodeId,
        owner: Option<ViewId>,
        opaque: bool,
        slot: Slot,
    ) -> ValueCursor {
        ValueCursor {
            location: Location::Raw(node),
            owner,
            opaque,
            slot,
            selected_token: None,
        }
    }

    fn reference_pointer(&mut self, reference: NodeId) -> Fallible<ValueCursor> {
        let Value::String(text) = self.nodes[reference].value else {
            self.report(
                self.nodes[reference].path.clone(),
                "reference",
                "$ref must be a local JSON Pointer fragment string.",
            );
            return Err(());
        };
        self.pointer(text, reference)
    }

    fn pointer(&mut self, text: &str, site: NodeId) -> Fallible<ValueCursor> {
        let segments = match parse_fragment(text) {
            Ok(segments) => segments,
            Err(message) => {
                self.report(self.nodes[site].path.clone(), "reference", message);
                return Err(());
            }
        };
        let mut cursor = self.namespace_cursor(0);
        for segment in segments {
            if !cursor.opaque && matches!(cursor.location, Location::Raw(_)) {
                cursor = self.resolve_pointer_target(cursor)?.cursor;
            }
            cursor = self.lookup(cursor, &segment, site)?;
        }
        Ok(cursor)
    }

    fn lookup(&mut self, cursor: ValueCursor, key: &str, site: NodeId) -> Fallible<ValueCursor> {
        if let Location::Namespace(view) = cursor.location {
            if self.kind(view)? == Kind::Group {
                if !key.starts_with('$') || key == "$root" {
                    let members = self.member(view, key)?;
                    if members.len() == 1 {
                        return Ok(self.namespace_cursor(members[0]));
                    }
                    if members.len() > 1 {
                        return self.lookup_error(
                            site,
                            "A pointer cannot select a duplicate object member.",
                        );
                    }
                    return self.lookup_error(site, "The referenced object member does not exist.");
                }
                let layers = self.group_layers(view)?;
                for &layer in layers.iter().rev() {
                    if self.has(layer, key) {
                        return self.raw_lookup(
                            self.raw_cursor(layer, Some(view), true, Slot::Literal),
                            key,
                            site,
                        );
                    }
                }
                return self.lookup_error(site, "The referenced object member does not exist.");
            }
            let node = self.definition(view);
            let mut result = self.raw_lookup(
                self.raw_cursor(node, Some(view), true, Slot::Literal),
                key,
                site,
            )?;
            if key == "$value" {
                result.opaque = false;
                result.slot = self.hint_type(view).map_or(Slot::Any, Slot::Typed);
                result.selected_token = Some(view);
            }
            return Ok(result);
        }
        self.raw_lookup(cursor, key, site)
    }

    fn raw_lookup(
        &mut self,
        cursor: ValueCursor,
        key: &str,
        site: NodeId,
    ) -> Fallible<ValueCursor> {
        let Location::Raw(node) = cursor.location else {
            unreachable!()
        };
        match self.nodes[node].value {
            Value::Object(members) => {
                let mut matches = members
                    .iter()
                    .enumerate()
                    .filter(|(_, (name, _))| name == key);
                if let Some((index, _)) = matches.next() {
                    if matches.next().is_none() {
                        return Ok(self.child(cursor, index));
                    }
                    return self
                        .lookup_error(site, "A pointer cannot select a duplicate object member.");
                }
                self.lookup_error(site, "The referenced object member does not exist.")
            }
            Value::Array(items) => {
                let valid = key == "0"
                    || (!key.starts_with('0')
                        && !key.is_empty()
                        && key.bytes().all(|byte| byte.is_ascii_digit()));
                if valid {
                    if let Ok(index) = key.parse::<usize>() {
                        if index < items.len() {
                            return Ok(self.child(cursor, index));
                        }
                    }
                }
                self.lookup_error(
                    site,
                    "The reference has an invalid or out-of-bounds array index.",
                )
            }
            _ => self.lookup_error(
                site,
                "The reference traverses a value that is not an object or array.",
            ),
        }
    }

    fn lookup_error<T>(&mut self, site: NodeId, message: &str) -> Fallible<T> {
        self.report(self.nodes[site].path.clone(), "reference", message);
        Err(())
    }

    fn curly_target(&mut self, text: &str, site: NodeId) -> Fallible<ValueCursor> {
        if !text.starts_with('{')
            || !text.ends_with('}')
            || text[1..text.len() - 1].contains(['{', '}'])
        {
            return self.lookup_error(
                site,
                "A curly reference must contain one complete token or group path.",
            );
        }
        let mut cursor = self.namespace_cursor(0);
        for segment in text[1..text.len() - 1].split('.') {
            let Location::Namespace(view) = cursor.location else {
                unreachable!()
            };
            if self.kind(view)? != Kind::Group {
                return self.lookup_error(
                    site,
                    "A curly reference must name a complete token or group.",
                );
            }
            if segment.starts_with('$') && segment != "$root" {
                return self.lookup_error(site, "A curly reference cannot select metadata.");
            }
            cursor = self.lookup(cursor, segment, site)?;
        }
        Ok(cursor)
    }

    pub(crate) fn value(&self, cursor: ValueCursor) -> &'a Value {
        let node = match cursor.location {
            Location::Raw(node) => node,
            Location::Namespace(view) => self.definition(view),
        };
        self.nodes[node].value
    }

    pub(crate) fn source_path(&self, cursor: ValueCursor) -> &str {
        let node = match cursor.location {
            Location::Raw(node) => node,
            Location::Namespace(view) => self.definition(view),
        };
        &self.nodes[node].path
    }

    pub(crate) fn token_path(&self, token: TokenId) -> &str {
        &self.nodes[self.definition(token)].path
    }

    /// Original alias/value declaration path, never an inherited placement.
    pub(crate) fn token_reference_path(&self, token: TokenId) -> &str {
        let node = self.definition(token);
        for key in ["$ref", "$value"] {
            if let Ok(Some(property)) = self.property(node, key) {
                return &self.nodes[property].path;
            }
        }
        &self.nodes[node].path
    }

    #[cfg(test)]
    pub(crate) fn effective_path(&self, token: TokenId) -> &str {
        &self.views[token].path
    }

    /// Selects an already inspected raw object's member or array element by
    /// source-order index. It does not interpret references or flatten arrays.
    pub(crate) fn child(&self, cursor: ValueCursor, index: usize) -> ValueCursor {
        let node = match cursor.location {
            Location::Raw(node) => node,
            Location::Namespace(view) => self.definition(view),
        };
        let key = match self.nodes[node].value {
            Value::Object(members) => Some(members[index].0.as_str()),
            _ => None,
        };
        let slot = child_slot(cursor.slot, key);
        self.raw_cursor(
            self.nodes[node].children[index],
            cursor.owner,
            cursor.opaque,
            slot,
        )
    }

    pub(crate) fn resolve_head(
        &mut self,
        cursor: ValueCursor,
        expected: Expected<'_>,
    ) -> Fallible<ResolvedValue> {
        if cursor.opaque || matches!(cursor.location, Location::Namespace(_)) {
            return Ok(ResolvedValue {
                cursor,
                alias: cursor.selected_token,
                curly_alias: false,
            });
        }
        let curly = !matches!(expected, Expected::Literal);
        let site = self.source_path(cursor).to_owned();
        self.begin(Operation::Head(cursor, curly), cursor.owner, site)?;
        let result = self.resolve_value_head(cursor, expected);
        self.end();
        result
    }

    fn resolve_value_head(
        &mut self,
        cursor: ValueCursor,
        expected: Expected<'_>,
    ) -> Fallible<ResolvedValue> {
        let Location::Raw(node) = cursor.location else {
            unreachable!()
        };
        let mut result = match self.nodes[node].value {
            Value::String(text)
                if !matches!(expected, Expected::Literal) && text.starts_with('{') =>
            {
                let target = self.curly_target(text, node)?;
                let Location::Namespace(token) = target.location else {
                    unreachable!()
                };
                if self.kind(token)? != Kind::Token {
                    return self.lookup_error(
                        node,
                        "A curly value reference must target a complete token.",
                    );
                }
                if let Expected::Type(expected) = expected {
                    if self.token_type(token)?.name != expected {
                        return self
                            .lookup_error(node, "The referenced token has an incompatible type.");
                    }
                }
                let mut value = self.token_value(token)?;
                value.alias = Some(token);
                value.curly_alias = true;
                value
            }
            Value::Object(members)
                if self.has(node, "$ref") && members.iter().all(|(key, _)| key == "$ref") =>
            {
                let reference = match self.property(node, "$ref") {
                    Ok(Some(reference)) => reference,
                    Ok(None) => unreachable!(),
                    Err(()) => {
                        self.report(
                            child_path(&self.nodes[node].path, "$ref"),
                            "structure",
                            "Conflicting duplicate $ref properties are ambiguous.",
                        );
                        return Err(());
                    }
                };
                let target = self.reference_pointer(reference)?;
                self.resolve_pointer_target(target)?
            }
            Value::String(_) if matches!(expected, Expected::Literal) => {
                // A pointer selecting an inner primitive string must not turn
                // its literal braces into a new alias in the caller's slot.
                let mut literal = cursor;
                literal.opaque = true;
                ResolvedValue {
                    cursor: literal,
                    alias: None,
                    curly_alias: false,
                }
            }
            _ => ResolvedValue {
                cursor,
                alias: None,
                curly_alias: false,
            },
        };
        if let Some(token) = cursor.selected_token {
            result.alias = Some(token);
            result.curly_alias = false;
            result.cursor.selected_token = None;
        }
        Ok(result)
    }

    fn resolve_pointer_target(&mut self, cursor: ValueCursor) -> Fallible<ResolvedValue> {
        if let Some(token) = cursor.selected_token {
            let mut value = self.token_value(token)?;
            value.alias = Some(token);
            value.curly_alias = false;
            value.cursor.selected_token = None;
            return Ok(value);
        }
        let mut value = self.resolve_head(cursor, cursor.slot.expected())?;
        value.curly_alias = false;
        Ok(value)
    }

    pub(crate) fn token_value(&mut self, token: TokenId) -> Fallible<ResolvedValue> {
        if let Some(result) = self.views[token].value {
            return result;
        }
        self.begin(
            Operation::Value(token),
            Some(token),
            self.reference_site(token),
        )?;
        let result = self.resolve_token_value(token);
        self.end();
        self.views[token].value = Some(result);
        result
    }

    fn resolve_token_value(&mut self, token: TokenId) -> Fallible<ResolvedValue> {
        if self.kind(token)? != Kind::Token {
            return Err(());
        }
        let node = self.definition(token);
        if let Some(value) = self.property(node, "$value")? {
            let slot = self.hint_type(token).map_or(Slot::Any, Slot::Typed);
            return self.resolve_head(
                self.raw_cursor(value, Some(token), false, slot),
                Expected::Any,
            );
        }
        let Some(reference) = self.property(node, "$ref")? else {
            return Err(());
        };
        let target = self.reference_pointer(reference)?;
        self.resolve_pointer_target(target)
    }

    fn declared_type(&self, node: NodeId) -> Fallible<Option<&'static str>> {
        let Some(property) = self.property(node, "$type")? else {
            return Ok(None);
        };
        let Value::String(name) = self.nodes[property].value else {
            return Err(());
        };
        SUPPORTED_TYPES
            .iter()
            .copied()
            .find(|candidate| *candidate == name)
            .map(Some)
            .ok_or(())
    }

    fn group_default(&mut self, mut parent: Option<ViewId>) -> Fallible<Option<&'static str>> {
        while let Some(group) = parent {
            for &layer in self.group_layers(group)?.iter().rev() {
                if self.has(layer, "$type") {
                    return self.declared_type(layer);
                }
            }
            parent = self.views[group].parent;
        }
        Ok(None)
    }

    fn hint_type(&mut self, token: TokenId) -> Option<&'static str> {
        if self.has(self.definition(token), "$type") {
            self.declared_type(self.definition(token)).ok().flatten()
        } else {
            self.group_default(self.views[token].parent).ok().flatten()
        }
    }

    pub(crate) fn effective_parent_type(
        &mut self,
        token: TokenId,
    ) -> Fallible<Option<&'static str>> {
        self.group_default(self.views[token].parent)
    }

    pub(crate) fn token_type(&mut self, token: TokenId) -> Fallible<TokenType> {
        if let Some(result) = self.views[token].token_type {
            return result;
        }
        self.begin(
            Operation::Type(token),
            Some(token),
            self.reference_site(token),
        )?;
        let result = self.determine_type(token);
        self.end();
        self.views[token].token_type = Some(result);
        result
    }

    fn determine_type(&mut self, token: TokenId) -> Fallible<TokenType> {
        let node = self.definition(token);
        if self.has(node, "$type") {
            return self
                .declared_type(node)?
                .map(|name| TokenType {
                    name,
                    origin: TypeOrigin::Explicit,
                })
                .ok_or(());
        }
        if let Some(target) = self.token_value(token)?.alias {
            let name = self.token_type(target)?.name;
            return Ok(TokenType {
                name,
                origin: TypeOrigin::Referenced(target),
            });
        }
        if let Some(name) = self.effective_parent_type(token)? {
            return Ok(TokenType {
                name,
                origin: TypeOrigin::Group,
            });
        }
        self.report(
            self.nodes[node].path.clone(),
            "type",
            "A token must have an explicit, referenced or inherited $type.",
        );
        Err(())
    }

    pub(crate) fn prepare(&mut self) {
        if !matches!(self.nodes[0].value, Value::Object(_)) {
            return;
        }
        // Force only original group envelopes first. No inherited descendant is
        // recursively visited until the finite containment/extension graph is checked.
        for view in self.original_views.clone() {
            if self.kind(view) == Ok(Kind::Group) {
                if let Some(parent) = self.views[view].parent {
                    let parent = self.definition(parent);
                    let child = self.definition(view);
                    self.group_edges.entry(parent).or_default().push(child);
                }
                if self.token_path(view).ends_with("/$root") {
                    self.report(
                        self.token_path(view).to_owned(),
                        "structure",
                        "$root must be a token, not a group.",
                    );
                }
                let _ = self.members(view);
            }
        }
        self.find_group_cycles();
        self.collect_tokens(0);
    }

    fn collect_tokens(&mut self, group: ViewId) {
        let Ok(layers) = self.group_layers(group) else {
            return;
        };
        if layers.iter().any(|node| self.cyclic_groups.contains(node)) {
            return;
        }
        let Ok(members) = self.members(group) else {
            return;
        };
        let mut local = Vec::new();
        let mut roots = Vec::new();
        let mut inherited = Vec::new();
        let mut groups = Vec::new();
        for (name, view) in members.iter().map(|(name, view)| (name, *view)) {
            match self.kind(view) {
                Ok(Kind::Token) => {
                    let source = self.definition(view);
                    if name == "$root" {
                        roots.push(view)
                    } else if self.views[group]
                        .definitions
                        .iter()
                        .any(|parent| self.nodes[*parent].children.contains(&source))
                    {
                        local.push(view);
                    } else {
                        inherited.push(view)
                    }
                }
                Ok(Kind::Group) => groups.push(view),
                Err(()) => {}
            }
        }
        self.tokens.extend(local);
        self.tokens.extend(roots);
        self.tokens.extend(inherited);
        for group in groups {
            self.collect_tokens(group);
        }
    }

    fn find_group_cycles(&mut self) {
        self.cyclic_groups = cycle_members(&self.group_edges, self.nodes.len());
        for node in self.cyclic_groups.clone() {
            let property = self
                .property(node, "$extends")
                .ok()
                .flatten()
                .or_else(|| self.property(node, "$ref").ok().flatten());
            let path = property.map_or_else(
                || self.nodes[node].path.clone(),
                |property| self.nodes[property].path.clone(),
            );
            self.report(
                path,
                "cycle",
                "Group inheritance and containment contain a cycle.",
            );
        }
    }

    pub(crate) fn tokens(&self) -> &[TokenId] {
        &self.tokens
    }

    /// Checks references without validating value shapes for focused resolver tests.
    /// Scanning is type-directed so primitive inner strings and extra
    /// singular-value metadata remain literal.
    #[cfg(test)]
    pub(crate) fn validate_references(&mut self) {
        for token in self.tokens.clone() {
            let value = self.token_value(token);
            let token_type = self.token_type(token);
            if let (Ok(value), Ok(token_type)) = (value, token_type) {
                if let Some(alias) = value.alias.filter(|_| value.curly_alias) {
                    if let Ok(alias_type) = self.token_type(alias) {
                        if alias_type.name != token_type.name {
                            self.report(
                                self.reference_site(token),
                                "reference",
                                "The referenced token has an incompatible type.",
                            );
                        }
                    }
                }
                self.scan(value.cursor, Slot::Typed(token_type.name));
            }
        }
    }

    #[cfg(test)]
    fn scan(&mut self, cursor: ValueCursor, slot: Slot) {
        if cursor.opaque {
            return;
        }
        if self
            .begin(
                Operation::Scan(cursor, slot),
                cursor.owner,
                self.source_path(cursor).to_owned(),
            )
            .is_err()
        {
            return;
        }
        if let Ok(value) = self.resolve_head(cursor, slot.expected()) {
            let cursor = value.cursor;
            match self.value(cursor) {
                Value::Array(values) => {
                    for index in 0..values.len() {
                        if let Some(child_slot) = scan_array_slot(slot) {
                            self.scan(self.child(cursor, index), child_slot);
                        }
                    }
                }
                Value::Object(members) => {
                    for (index, (name, _)) in members.iter().enumerate() {
                        if let Some(child_slot) = scan_object_slot(slot, name) {
                            self.scan(self.child(cursor, index), child_slot);
                        }
                    }
                }
                _ => {}
            }
        }
        self.end();
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

fn child_slot(slot: Slot, key: Option<&str>) -> Slot {
    match key {
        Some(key) => scan_object_slot(slot, key).unwrap_or(Slot::Literal),
        None => scan_array_slot(slot).unwrap_or(Slot::Literal),
    }
}

fn scan_array_slot(slot: Slot) -> Option<Slot> {
    match slot {
        Slot::Typed("shadow") => Some(Slot::ShadowItem),
        Slot::Typed("gradient") => Some(Slot::GradientStop),
        Slot::Dimensions => Some(Slot::Typed("dimension")),
        Slot::Typed("fontFamily" | "cubicBezier") | Slot::Literal => Some(Slot::Literal),
        _ => None,
    }
}

fn scan_object_slot(slot: Slot, key: &str) -> Option<Slot> {
    let name = match (slot, key) {
        (Slot::Typed("border"), "color") => "color",
        (Slot::Typed("border"), "width") => "dimension",
        (Slot::Typed("border"), "style") => "strokeStyle",
        (Slot::Typed("transition"), "duration" | "delay") => "duration",
        (Slot::Typed("transition"), "timingFunction") => "cubicBezier",
        (Slot::Typed("shadow") | Slot::ShadowItem, "color") => "color",
        (Slot::Typed("shadow") | Slot::ShadowItem, "offsetX" | "offsetY" | "blur" | "spread") => {
            "dimension"
        }
        (Slot::Typed("shadow") | Slot::ShadowItem, "inset") => return Some(Slot::Literal),
        (Slot::GradientStop, "color") => "color",
        (Slot::GradientStop, "position") => "number",
        (Slot::Typed("typography"), "fontFamily") => "fontFamily",
        (Slot::Typed("typography"), "fontSize" | "letterSpacing") => "dimension",
        (Slot::Typed("typography"), "fontWeight") => "fontWeight",
        (Slot::Typed("typography"), "lineHeight") => "number",
        (Slot::Typed("strokeStyle"), "dashArray") => return Some(Slot::Dimensions),
        (Slot::Typed("strokeStyle"), "lineCap") => return Some(Slot::Literal),
        (Slot::Typed("dimension" | "duration"), "value" | "unit")
        | (Slot::Typed("color"), "colorSpace" | "components" | "alpha" | "hex") => {
            return Some(Slot::Literal);
        }
        _ => return None,
    };
    Some(Slot::Typed(name))
}

fn parse_fragment(text: &str) -> Result<Vec<String>, &'static str> {
    let Some(fragment) = text.strip_prefix('#') else {
        return Err("A reference must be a local JSON Pointer fragment beginning with #.");
    };
    let mut decoded = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(hex_digit)
                .ok_or("A pointer fragment contains an invalid percent escape.")?;
            let low = bytes
                .next()
                .and_then(hex_digit)
                .ok_or("A pointer fragment contains an invalid percent escape.")?;
            decoded.push(high * 16 + low);
        } else {
            decoded.push(byte);
        }
    }
    let decoded =
        String::from_utf8(decoded).map_err(|_| "A pointer fragment must decode to valid UTF-8.")?;
    if decoded.is_empty() {
        return Ok(Vec::new());
    }
    let Some(pointer) = decoded.strip_prefix('/') else {
        return Err("A non-root JSON Pointer must begin with /.");
    };
    pointer
        .split('/')
        .map(|segment| {
            let mut key = String::with_capacity(segment.len());
            let mut characters = segment.chars();
            while let Some(character) = characters.next() {
                if character == '~' {
                    match characters.next() {
                        Some('0') => key.push('~'),
                        Some('1') => key.push('/'),
                        _ => return Err("A JSON Pointer contains an invalid ~ escape."),
                    }
                } else {
                    key.push(character);
                }
            }
            Ok(key)
        })
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// Tarjan's algorithm keeps containment-cycle detection linear in the finite
// source graph, rather than following ever-growing inherited placement paths.
fn cycle_members(graph: &HashMap<NodeId, Vec<NodeId>>, node_count: usize) -> HashSet<NodeId> {
    struct Search<'a> {
        graph: &'a HashMap<NodeId, Vec<NodeId>>,
        indices: Vec<usize>,
        low: Vec<usize>,
        stack: Vec<NodeId>,
        on_stack: Vec<bool>,
        next: usize,
        cyclic: HashSet<NodeId>,
    }
    impl Search<'_> {
        fn visit(&mut self, node: NodeId) {
            self.indices[node] = self.next;
            self.low[node] = self.next;
            self.next += 1;
            self.stack.push(node);
            self.on_stack[node] = true;
            let graph = self.graph;
            if let Some(edges) = graph.get(&node) {
                for &next in edges {
                    if self.indices[next] == usize::MAX {
                        self.visit(next);
                        self.low[node] = self.low[node].min(self.low[next]);
                    } else if self.on_stack[next] {
                        self.low[node] = self.low[node].min(self.indices[next]);
                    }
                }
            }
            if self.low[node] == self.indices[node] {
                let mut component = Vec::new();
                loop {
                    let member = self.stack.pop().expect("an open component has a root");
                    self.on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                if component.len() > 1
                    || graph.get(&node).is_some_and(|edges| edges.contains(&node))
                {
                    self.cyclic.extend(component);
                }
            }
        }
    }
    let mut search = Search {
        graph,
        indices: vec![usize::MAX; node_count],
        low: vec![0; node_count],
        stack: Vec::new(),
        on_stack: vec![false; node_count],
        next: 0,
        cyclic: HashSet::new(),
    };
    for &node in graph.keys() {
        if search.indices[node] == usize::MAX {
            search.visit(node);
        }
    }
    search.cyclic
}

#[cfg(test)]
mod tests;
