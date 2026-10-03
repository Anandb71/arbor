//! The type of a Rust method call's receiver, where the code states it.
//!
//! `draft.run_started()` names no type, so the graph builder used to resolve
//! it by method name alone. When several types define `run_started` that is
//! ambiguous, and the call was dropped: `arbor callers` then reported none.
//! Usually the type is written nearby, though:
//!
//! - a parameter: `fn on_key(draft: &mut Draft)`, `state: State<'_, AppState>`
//! - a `let`: `let d: Draft = ...`, `let d = Draft::new()?`, `Draft { .. }`
//! - an `if let Some(d) = state.draft.as_mut()` or a typed closure parameter
//! - a field: `state.draft.lock().unwrap()` reaches `AppState`'s `draft`
//!
//! The answer is a type path: a base type plus any fields read from it
//! (`AppState.draft`). Field types live on struct nodes (see
//! [`crate::node::field_ref`]), so the builder can follow a path into a
//! struct defined in another file. References, smart pointers, locks,
//! `Option` and collections are seen through, because a method a project
//! defines is reached on what they contain.
//!
//! Anything uncertain gives no answer and the call falls back to name-based
//! resolution: a name bound to two different types in one function, a
//! generic parameter without a trait bound, or an expression whose type
//! depends on a function's return value.

use std::collections::HashMap;

use tree_sitter::Node;

/// Types a project method is called *through*: the method is defined on the
/// last type argument (`Arc<Mutex<Draft>>` reaches `Draft`).
const WRAPPERS: &[&str] = &[
    "Arc",
    "BTreeMap",
    "BTreeSet",
    "Box",
    "Cell",
    "Cow",
    "HashMap",
    "HashSet",
    "IndexMap",
    "LinkedList",
    "Mutex",
    "MutexGuard",
    "Option",
    "OnceCell",
    "OnceLock",
    "Pin",
    "Rc",
    "Ref",
    "RefCell",
    "RefMut",
    "RwLock",
    "RwLockReadGuard",
    "RwLockWriteGuard",
    "State",
    "Vec",
    "VecDeque",
    "Weak",
];

/// Methods that hand back what their receiver wraps: `lock().unwrap()`,
/// `as_mut()`, `get_mut(&id)`. Their result has the receiver's (stripped) type.
const PASS_THROUGH: &[&str] = &[
    "as_deref",
    "as_deref_mut",
    "as_mut",
    "as_ref",
    "blocking_lock",
    "blocking_read",
    "blocking_write",
    "borrow",
    "borrow_mut",
    "clone",
    "deref",
    "deref_mut",
    "expect",
    "first",
    "first_mut",
    "get",
    "get_mut",
    "get_or_init",
    "inner",
    "last",
    "last_mut",
    "lock",
    "read",
    "to_owned",
    "try_borrow",
    "try_borrow_mut",
    "try_lock",
    "try_read",
    "try_write",
    "unwrap",
    "unwrap_or_default",
    "upgrade",
    "write",
];

/// Associated functions that return `Self` (possibly in a `Result`, which `?`
/// or `unwrap` sees through).
fn is_constructor(name: &str) -> bool {
    matches!(name, "new" | "default" | "from" | "try_from" | "try_new")
        || ["new_", "with_", "from_", "try_from_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Receiver types known inside one function.
pub(super) struct Receivers<'s> {
    source: &'s str,
    /// The `impl` target, for `self`, `Self` and `Self::new()`.
    self_type: Option<String>,
    /// Generic parameters, mapped to their first trait bound if any.
    generics: HashMap<String, Option<String>>,
    /// Bound names. `None` marks a name seen with two different types.
    bindings: HashMap<String, Option<String>>,
}

impl<'s> Receivers<'s> {
    /// Collects the parameters and local bindings of `function`.
    pub(super) fn of_function(function: &Node, source: &'s str, self_type: Option<&str>) -> Self {
        let mut receivers = Self {
            source,
            self_type: self_type.map(str::to_string),
            generics: HashMap::new(),
            bindings: HashMap::new(),
        };
        receivers.collect_generics(function);
        if let Some(parameters) = function.child_by_field_name("parameters") {
            receivers.bind_parameters(&parameters);
        }
        if let Some(body) = function.child_by_field_name("body") {
            receivers.collect_bindings(&body);
        }
        receivers
    }

    /// The receiver type path for `value` in `value.method()`, if known.
    pub(super) fn type_of(&self, value: &Node) -> Option<String> {
        stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || self.expression_type(value))
    }

    fn text(&self, node: &Node) -> &'s str {
        self.source.get(node.byte_range()).unwrap_or_default()
    }

    fn expression_type(&self, node: &Node) -> Option<String> {
        match node.kind() {
            "self" => self.self_type.clone(),
            "identifier" => self.bindings.get(self.text(node)).cloned().flatten(),
            "field_expression" => {
                let field = node.child_by_field_name("field")?;
                // `pair.0` has no field name to follow.
                if field.kind() != "field_identifier" {
                    return None;
                }
                let base = self.expression_type(&node.child_by_field_name("value")?)?;
                Some(format!("{base}.{}", self.text(&field)))
            }
            "call_expression" => self.call_type(node),
            "try_expression" | "parenthesized_expression" => {
                self.expression_type(&node.named_child(0)?)
            }
            "reference_expression" => self.expression_type(&node.child_by_field_name("value")?),
            "unary_expression" if self.text(node).starts_with('*') => {
                self.expression_type(&node.named_child(0)?)
            }
            "index_expression" => self.expression_type(&node.named_child(0)?),
            "struct_expression" => {
                let name = node.child_by_field_name("name")?;
                self.type_name_text(&name)
            }
            _ => None,
        }
    }

    fn call_type(&self, call: &Node) -> Option<String> {
        let function = call.child_by_field_name("function")?;
        match function.kind() {
            // `state.draft.lock()`, `slot.as_mut()`
            "field_expression" => {
                let method = function.child_by_field_name("field")?;
                if PASS_THROUGH.contains(&self.text(&method)) {
                    self.expression_type(&function.child_by_field_name("value")?)
                } else {
                    None
                }
            }
            // `app.state::<AppState>()` (Tauri's managed state)
            "generic_function" => {
                let inner = function.child_by_field_name("function")?;
                let method = inner.child_by_field_name("field")?;
                if !matches!(self.text(&method), "state" | "try_state") {
                    return None;
                }
                let arguments = function.child_by_field_name("type_arguments")?;
                let argument = arguments.named_child(0)?;
                self.core_type(&argument)
            }
            // `Draft::new()`, `Self::default()`
            "scoped_identifier" => {
                let name = function.child_by_field_name("name")?;
                if !is_constructor(self.text(&name)) {
                    return None;
                }
                let path = function.child_by_field_name("path")?;
                self.type_name_text(&path)
            }
            _ => None,
        }
    }

    /// `Draft`, `Self`, or the last segment of `crate::draft::Draft`, when it
    /// looks like a type and isn't an unbounded generic.
    fn type_name_text(&self, node: &Node) -> Option<String> {
        let text = self.text(node).trim();
        let last = text.rsplit("::").next().unwrap_or(text).trim();
        let last = last.split('<').next().unwrap_or(last).trim();
        self.named_type(last)
    }

    fn named_type(&self, name: &str) -> Option<String> {
        if name == "Self" {
            return self.self_type.clone();
        }
        if let Some(bound) = self.generics.get(name) {
            return bound.clone();
        }
        let starts_upper = name.starts_with(|c: char| c.is_ascii_uppercase());
        let identifier = name.chars().all(|c| c.is_alphanumeric() || c == '_');
        (starts_upper && identifier).then(|| name.to_string())
    }

    /// The type a method call on a value of type `node` reaches.
    pub(super) fn core_type(&self, node: &Node) -> Option<String> {
        match node.kind() {
            "type_identifier" | "scoped_type_identifier" => self.type_name_text(node),
            "reference_type" | "pointer_type" => self.core_type(&node.child_by_field_name("type")?),
            "generic_type" => {
                let base = node.child_by_field_name("type")?;
                let base_name = self.text(&base).rsplit("::").next().unwrap_or_default();
                if WRAPPERS.contains(&base_name) {
                    let arguments = node.child_by_field_name("type_arguments")?;
                    let last = (0..arguments.named_child_count())
                        .filter_map(|i| arguments.named_child(i))
                        .filter(|argument| argument.kind() != "lifetime")
                        .last()?;
                    self.core_type(&last)
                } else {
                    self.type_name_text(&base)
                }
            }
            // `&mut dyn RunObserver`, `impl RunObserver`: the trait.
            "dynamic_type" | "abstract_type" => {
                let bound = node.child_by_field_name("trait").or_else(|| node.named_child(0))?;
                self.core_type(&bound)
            }
            "array_type" => self.core_type(&node.child_by_field_name("element")?),
            _ => None,
        }
    }

    fn collect_generics(&mut self, function: &Node) {
        if let Some(parameters) = function.child_by_field_name("type_parameters") {
            for i in 0..parameters.named_child_count() {
                let Some(parameter) = parameters.named_child(i) else {
                    continue;
                };
                match parameter.kind() {
                    "type_identifier" => {
                        self.generics.insert(self.text(&parameter).to_string(), None);
                    }
                    "constrained_type_parameter" => {
                        let Some(left) = parameter.child_by_field_name("left") else {
                            continue;
                        };
                        let bound = parameter
                            .child_by_field_name("bounds")
                            .and_then(|bounds| bounds.named_child(0))
                            .and_then(|first| self.bound_name(&first));
                        self.generics.insert(self.text(&left).to_string(), bound);
                    }
                    _ => {}
                }
            }
        }
        // `where T: RunObserver`
        let where_clause = (0..function.child_count())
            .filter_map(|i| function.child(i))
            .find(|child| child.kind() == "where_clause");
        if let Some(clause) = where_clause {
            for i in 0..clause.named_child_count() {
                let Some(predicate) = clause.named_child(i) else {
                    continue;
                };
                let (Some(left), Some(bounds)) = (
                    predicate.child_by_field_name("left"),
                    predicate.child_by_field_name("bounds"),
                ) else {
                    continue;
                };
                let name = self.text(&left).to_string();
                let bound = bounds.named_child(0).and_then(|first| self.bound_name(&first));
                if let Some(slot) = self.generics.get_mut(&name) {
                    if slot.is_none() {
                        *slot = bound;
                    }
                }
            }
        }
    }

    /// A trait bound's name: `RunObserver`, `observer::RunObserver`.
    fn bound_name(&self, bound: &Node) -> Option<String> {
        match bound.kind() {
            "type_identifier" | "scoped_type_identifier" | "generic_type" => {
                let text = self.text(bound);
                let last = text.rsplit("::").next().unwrap_or(text);
                let last = last.split('<').next().unwrap_or(last).trim();
                let upper = last.starts_with(|c: char| c.is_ascii_uppercase());
                upper.then(|| last.to_string())
            }
            _ => None,
        }
    }

    fn bind(&mut self, name: &str, type_path: Option<String>) {
        if name.is_empty() || name == "_" {
            return;
        }
        match self.bindings.get(name) {
            // Shadowed with a different (or unknown) type: no longer certain.
            Some(existing) if *existing != type_path => {
                self.bindings.insert(name.to_string(), None);
            }
            Some(_) => {}
            None => {
                self.bindings.insert(name.to_string(), type_path);
            }
        }
    }

    fn bind_parameters(&mut self, parameters: &Node) {
        for i in 0..parameters.named_child_count() {
            let Some(parameter) = parameters.named_child(i) else {
                continue;
            };
            if parameter.kind() == "parameter" {
                let pattern = parameter.child_by_field_name("pattern");
                let type_path = parameter
                    .child_by_field_name("type")
                    .and_then(|t| self.core_type(&t));
                if let Some(pattern) = pattern {
                    self.bind_pattern(&pattern, type_path);
                }
            } else if parameter.kind() == "identifier" {
                // An untyped closure parameter: `|d| d.run()`.
                let name = self.text(&parameter).to_string();
                self.bind(&name, None);
            }
        }
    }

    /// Binds the names a pattern introduces. Only a plain name, `mut name`,
    /// `ref name` or `Some(name)` / `Ok(name)` receive the type; anything
    /// destructured is marked unknown.
    fn bind_pattern(&mut self, pattern: &Node, type_path: Option<String>) {
        match pattern.kind() {
            "identifier" => {
                let name = self.text(pattern).to_string();
                self.bind(&name, type_path);
            }
            "mut_pattern" | "ref_pattern" => {
                if let Some(inner) = pattern.named_child(pattern.named_child_count().saturating_sub(1)) {
                    self.bind_pattern(&inner, type_path);
                }
            }
            "tuple_struct_pattern" => {
                let wrapper = pattern
                    .child_by_field_name("type")
                    .map(|t| self.text(&t))
                    .unwrap_or_default();
                let passes = matches!(wrapper, "Some" | "Ok");
                for i in 0..pattern.named_child_count() {
                    let Some(child) = pattern.named_child(i) else {
                        continue;
                    };
                    if Some(child) == pattern.child_by_field_name("type") {
                        continue;
                    }
                    let bound = if passes && pattern.named_child_count() == 2 {
                        type_path.clone()
                    } else {
                        None
                    };
                    self.bind_pattern(&child, bound);
                }
            }
            _ => self.bind_names_unknown(pattern),
        }
    }

    fn bind_names_unknown(&mut self, node: &Node) {
        if node.kind() == "identifier" {
            let name = self.text(node).to_string();
            self.bind(&name, None);
            return;
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                self.bind_names_unknown(&child);
            }
        }
    }

    /// Walks a body in source order so a binding can use earlier ones.
    fn collect_bindings(&mut self, node: &Node) {
        stacker::maybe_grow(64 * 1024, 4 * 1024 * 1024, || match node.kind() {
            // A nested item is its own function with its own scope.
            "function_item" | "impl_item" | "trait_item" | "mod_item" => {}
            "let_declaration" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.collect_bindings(&value);
                }
                let type_path = match node.child_by_field_name("type") {
                    Some(annotation) => self.core_type(&annotation),
                    None => node
                        .child_by_field_name("value")
                        .and_then(|value| self.expression_type(&value)),
                };
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_pattern(&pattern, type_path);
                }
            }
            "let_condition" => {
                let value = node.child_by_field_name("value");
                if let Some(value) = &value {
                    self.collect_bindings(value);
                }
                let type_path = value.and_then(|value| self.expression_type(&value));
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_pattern(&pattern, type_path);
                }
            }
            "closure_expression" => {
                if let Some(parameters) = node.child_by_field_name("parameters") {
                    self.bind_parameters(&parameters);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.collect_bindings(&body);
                }
            }
            // `for d in drafts.iter_mut()`: the element type isn't tracked.
            "for_expression" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_names_unknown(&pattern);
                }
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if Some(child) != node.child_by_field_name("pattern") {
                            self.collect_bindings(&child);
                        }
                    }
                }
            }
            "match_arm" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_names_unknown(&pattern);
                }
                if let Some(value) = node.child_by_field_name("value") {
                    self.collect_bindings(&value);
                }
            }
            _ => {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        self.collect_bindings(&child);
                    }
                }
            }
        })
    }
}

/// `field:name:Type` references for a struct's named fields.
pub(super) fn struct_field_refs(item: &Node, source: &str, self_type: &str) -> Vec<String> {
    let Some(body) = item.child_by_field_name("body") else {
        return Vec::new();
    };
    if body.kind() != "field_declaration_list" {
        return Vec::new();
    }
    let mut generics = HashMap::new();
    if let Some(parameters) = item.child_by_field_name("type_parameters") {
        for i in 0..parameters.named_child_count() {
            if let Some(parameter) = parameters.named_child(i) {
                let name = parameter
                    .child_by_field_name("left")
                    .unwrap_or(parameter);
                generics.insert(source[name.byte_range()].to_string(), None);
            }
        }
    }
    let receivers = Receivers {
        source,
        self_type: Some(self_type.to_string()),
        generics,
        bindings: HashMap::new(),
    };
    let mut refs = Vec::new();
    for i in 0..body.named_child_count() {
        let Some(field) = body.named_child(i) else {
            continue;
        };
        if field.kind() != "field_declaration" {
            continue;
        }
        let (Some(name), Some(ty)) = (
            field.child_by_field_name("name"),
            field.child_by_field_name("type"),
        ) else {
            continue;
        };
        if let Some(type_name) = receivers.core_type(&ty) {
            refs.push(crate::node::field_ref(&source[name.byte_range()], &type_name));
        }
    }
    refs
}
