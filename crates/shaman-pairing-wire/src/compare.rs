//! Judging a wire's schema against the one it changes: what an older reader
//! could not survive, and whether the version and floor moved as they must
//! (ADR-0078).
//!
//! Types are compared by what they look like on the wire, never by their Rust
//! names, so renaming a type changes nothing here. Each change is found where
//! a payload leads to it, from a route, header or message, and a type reached
//! from several places is compared once.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    rc::Rc,
};

use serde_json::{Map, Value, json};

/// What changed between two schemas of a wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    pub kind: ChangeKind,
    /// Where it changed: a route, header or message, or a type's field.
    pub at: String,
    /// How it changed, where more is to be said than its kind.
    pub detail: String,
    /// The route, header or message the change was first reached from.
    pub via: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChangeKind {
    /// Something an older side speaks is gone.
    Removed,
    /// Something an older side speaks goes by another name.
    Renamed,
    /// Something an older side speaks has another shape.
    Retyped,
    /// A reader requires what an older writer does not send.
    NewlyRequired,
    /// A reader requires, or reads otherwise, what an older writer leaves out.
    DroppedDefault,
    /// Something new that an older side does without: a field it may leave
    /// out, a kind, a route.
    Added,
}

impl ChangeKind {
    /// Whether an older side could fail to survive it.
    pub fn breaks(self) -> bool {
        self != Self::Added
    }

    fn says(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Renamed => "renamed",
            Self::Retyped => "retyped",
            Self::NewlyRequired => "newly required",
            Self::DroppedDefault => "dropped its default",
            Self::Added => "added",
        }
    }
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.says(), self.at)?;
        if !self.detail.is_empty() {
            write!(f, " ({})", self.detail)?;
        }
        if self.via != self.at {
            write!(f, ", reached by {}", self.via)?;
        }
        Ok(())
    }
}

/// The version and floor a schema was built at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Numbers {
    pub version: u32,
    pub floor: u32,
}

/// A schema judged against the one it changes.
#[derive(Debug)]
pub struct Verdict {
    pub base: Numbers,
    pub current: Numbers,
    pub changes: Vec<Change>,
}

impl Verdict {
    /// The changes an older side could not survive.
    pub fn breaks(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|change| change.kind.breaks())
    }

    /// Why the change may not land, each said once: none where it may.
    pub fn problems(&self) -> Vec<String> {
        let Self { base, current, .. } = *self;
        let mut problems = Vec::new();
        if current.version < base.version {
            problems.push(format!(
                "PROTOCOL_VERSION went back from {} to {}",
                base.version, current.version
            ));
        }
        if current.floor < base.floor {
            problems.push(format!(
                "PROTOCOL_FLOOR went down from {} to {}; a floor only rises",
                base.floor, current.floor
            ));
        }
        if current.floor > current.version {
            problems.push(format!(
                "PROTOCOL_FLOOR {} is above PROTOCOL_VERSION {}",
                current.floor, current.version
            ));
        }
        if !self.changes.is_empty() && current.version == base.version {
            problems.push(format!(
                "the Pairing wire changed and PROTOCOL_VERSION did not; move it past {}",
                base.version
            ));
        }
        let breaks = self.breaks().count();
        if breaks > 0 && current.floor <= base.floor {
            problems.push(format!(
                "{breaks} change(s) above would break a side older than this one: make them by \
                 addition, or raise PROTOCOL_FLOOR to this change's PROTOCOL_VERSION in this \
                 same change, as a decision recorded for it"
            ));
        } else if breaks > 0 && current.floor < current.version {
            problems.push(format!(
                "{breaks} change(s) above would break a side older than this one, which a floor \
                 of {} still speaks with: raise PROTOCOL_FLOOR to {}",
                current.floor, current.version
            ));
        }
        problems
    }

    pub fn passes(&self) -> bool {
        self.problems().is_empty()
    }

    /// The whole judgement, as CI says it.
    pub fn report(&self) -> String {
        let mut report = format!(
            "Pairing wire: version {} -> {}, floor {} -> {}\n",
            self.base.version, self.current.version, self.base.floor, self.current.floor
        );
        if self.changes.is_empty() {
            report.push_str("No change to its shape.\n");
        }
        for change in &self.changes {
            let mark = if change.kind.breaks() { "x" } else { "+" };
            report.push_str(&format!("{mark} {change}\n"));
        }
        for problem in self.problems() {
            report.push_str(&format!("error: {problem}\n"));
        }
        report
    }
}

/// Judges `current` against `base`, the schema it changes.
pub fn judge(base: &Value, current: &Value) -> Result<Verdict, String> {
    let base_numbers = numbers(base, "base")?;
    let current_numbers = numbers(current, "current")?;
    for (document, which) in [(base, "base"), (current, "current")] {
        if let Some(keyword) = unread_keyword(document) {
            return Err(format!(
                "the {which} schema uses `{keyword}`, which this check does not read; teach \
                 `shaman-pairing-wire` what a change to it means before relying on it"
            ));
        }
    }
    let mut comparer = Comparer {
        old_defs: Rc::new(section_of(base, "$defs")),
        new_defs: Rc::new(section_of(current, "$defs")),
        seen: HashSet::new(),
        changes: Vec::new(),
        via: String::new(),
    };
    for (section, what) in [("headers", "header"), ("messages", "message")] {
        comparer.named(
            &section_of(base, section),
            &section_of(current, section),
            &|name| format!("{what} {name}"),
        );
    }
    for (section, prefix) in [("listener", "listener "), ("api", "")] {
        comparer.routes(
            &section_of(base, section),
            &section_of(current, section),
            prefix,
        );
    }
    Ok(Verdict {
        base: base_numbers,
        current: current_numbers,
        changes: comparer.changes,
    })
}

/// The first keyword anywhere in `document`'s schemas that this check does
/// not read, so a schema that came to use one fails rather than passes
/// unread. Bounds on a number or a string's length say nothing a format does
/// not, and are passed over.
fn unread_keyword(document: &Value) -> Option<String> {
    fn schema(value: &Value) -> Option<String> {
        let Value::Object(schema_map) = value else {
            return None;
        };
        for (keyword, value) in schema_map {
            match keyword.as_str() {
                "type" | "required" | "const" | "default" | "enum" | "format" | "minimum"
                | "maximum" | "minItems" | "maxItems" | "minLength" | "maxLength" | "$ref" => {}
                "properties" => {
                    for property in value.as_object()?.values() {
                        if let Some(keyword) = schema(property) {
                            return Some(keyword);
                        }
                    }
                }
                "items" | "additionalProperties" => {
                    if let Some(keyword) = schema(value) {
                        return Some(keyword);
                    }
                }
                "anyOf" | "oneOf" | "prefixItems" => {
                    for choice in value.as_array()? {
                        if let Some(keyword) = schema(choice) {
                            return Some(keyword);
                        }
                    }
                }
                keyword => return Some(keyword.to_owned()),
            }
        }
        None
    }
    let mut schemas: Vec<&Value> = Vec::new();
    for section in ["$defs", "headers", "messages"] {
        schemas.extend(
            document
                .get(section)
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(Map::values),
        );
    }
    for section in ["listener", "api"] {
        for route in document
            .get(section)
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(Map::values)
        {
            schemas.extend(route.get("query"));
            for part in ["body", "answer"].iter().filter_map(|part| route.get(part)) {
                schemas.extend(part.get("json"));
                schemas.extend(part.get("lines"));
                schemas.extend(
                    part.get("events")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(Map::values),
                );
            }
        }
    }
    schemas.into_iter().find_map(schema)
}

fn numbers(document: &Value, which: &str) -> Result<Numbers, String> {
    let number = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| format!("the {which} schema states no {key}"))
    };
    Ok(Numbers {
        version: number("protocol_version")?,
        floor: number("protocol_floor")?,
    })
}

fn section_of(document: &Value, section: &str) -> Map<String, Value> {
    document
        .get(section)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

struct Comparer {
    old_defs: Rc<Map<String, Value>>,
    new_defs: Rc<Map<String, Value>>,
    /// The pairs of definitions already compared, so each is compared once and
    /// a type that holds itself ends.
    seen: HashSet<(String, String)>,
    changes: Vec<Change>,
    via: String,
}

impl Comparer {
    fn change(&mut self, kind: ChangeKind, at: &str, detail: impl Into<String>) {
        self.changes.push(Change {
            kind,
            at: at.to_owned(),
            detail: detail.into(),
            via: self.via.clone(),
        });
    }

    /// Compares two maps of named schemas, such as headers or events.
    fn named(
        &mut self,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
        label: &dyn Fn(&str) -> String,
    ) {
        let (removed, added) = unmatched(old, new);
        self.removed_and_added(
            &removed,
            &added,
            label,
            |old, new| old == new,
            |_| (ChangeKind::Added, ""),
        );
        for (name, old_schema) in old {
            if let Some(new_schema) = new.get(name) {
                let at = label(name);
                self.via = at.clone();
                self.schema(old_schema, new_schema, &at);
            }
        }
    }

    fn routes(&mut self, old: &Map<String, Value>, new: &Map<String, Value>, prefix: &str) {
        let (removed, added) = unmatched(old, new);
        self.removed_and_added(
            &removed,
            &added,
            |route| format!("{prefix}{route}"),
            |old, new| old == new,
            |_| (ChangeKind::Added, ""),
        );
        for (route, old_parts) in old {
            let Some(new_parts) = new.get(route) else {
                continue;
            };
            for part in ["query", "body", "answer"] {
                let at = format!("{prefix}{route} {part}");
                self.via = at.clone();
                match (old_parts.get(part), new_parts.get(part)) {
                    (None, None) => {}
                    // A route that reads no query reads an empty one.
                    (old_part, new_part) if part == "query" => {
                        let nothing = json!({ "type": "object" });
                        self.schema(
                            old_part.unwrap_or(&nothing),
                            new_part.unwrap_or(&nothing),
                            &at,
                        );
                    }
                    (Some(_), None) => self.change(ChangeKind::Removed, &at, ""),
                    (None, Some(_)) if part == "answer" => self.change(ChangeKind::Added, &at, ""),
                    (None, Some(_)) => {
                        self.change(ChangeKind::Retyped, &at, "takes a body it took none of");
                    }
                    (Some(old_part), Some(new_part)) => self.part(old_part, new_part, &at),
                }
            }
        }
    }

    fn part(&mut self, old: &Value, new: &Value, at: &str) {
        let kind = |part: &Value| {
            part.as_object()
                .and_then(|part| part.keys().next().cloned())
                .unwrap_or_default()
        };
        let (old_kind, new_kind) = (kind(old), kind(new));
        if old_kind != new_kind {
            let detail = format!("was {old_kind}, is {new_kind}");
            return self.change(ChangeKind::Retyped, at, detail);
        }
        let (old, new) = (&old[&old_kind], &new[&new_kind]);
        match old_kind.as_str() {
            "bytes" if old != new => {
                self.change(ChangeKind::Retyped, at, format!("was {old}, is {new}"));
            }
            "bytes" => {}
            "events" => {
                let empty = Map::new();
                let (old, new) = (
                    old.as_object().unwrap_or(&empty),
                    new.as_object().unwrap_or(&empty),
                );
                let route = at.to_owned();
                self.named(old, new, &|event| format!("{route} event {event}"));
                self.via = route;
            }
            _ => self.schema(old, new, at),
        }
    }

    /// Reports what is in `old` alone and `new` alone: one of each as a rename
    /// where `renamed` says their bodies match, and each added by the kind
    /// `added_kind` gives its name.
    fn removed_and_added(
        &mut self,
        removed: &[(&str, &Value)],
        added: &[(&str, &Value)],
        label: impl Fn(&str) -> String,
        renamed: impl Fn(&Value, &Value) -> bool,
        added_kind: impl Fn(&str) -> (ChangeKind, &'static str),
    ) {
        if let ([(old_name, old_body)], [(new_name, new_body)]) = (removed, added)
            && renamed(old_body, new_body)
        {
            let detail = format!("now {}", label(new_name));
            return self.change(ChangeKind::Renamed, &label(old_name), detail);
        }
        for (name, _) in removed {
            self.change(ChangeKind::Removed, &label(name), "");
        }
        for (name, _) in added {
            let (kind, detail) = added_kind(name);
            self.change(kind, &label(name), detail);
        }
    }

    fn schema(&mut self, old: &Value, new: &Value, at: &str) {
        let (old_defs, new_defs) = (Rc::clone(&self.old_defs), Rc::clone(&self.new_defs));
        let (old, old_name) = resolve(&old_defs, old);
        let (new, new_name) = resolve(&new_defs, new);
        let mut at = at.to_owned();
        if let (Some(old_name), Some(new_name)) = (old_name, new_name)
            && !self.seen.insert((old_name.to_owned(), new_name.to_owned()))
        {
            return;
        }
        if let Some(new_name) = new_name {
            new_name.clone_into(&mut at);
        }
        let (old_null, old) = without_null(old);
        let (new_null, new) = without_null(new);
        if old_null != new_null {
            let detail = if new_null {
                "may now be null"
            } else {
                "may no longer be null"
            };
            self.change(ChangeKind::Retyped, &at, detail);
        }
        if old.get("$ref").is_some() || new.get("$ref").is_some() {
            return self.schema(&old, &new, &at);
        }
        let old_alternatives = alternatives(&old_defs, &old);
        let new_alternatives = alternatives(&new_defs, &new);
        if let ([(_, old_one)], [(_, new_one)]) = (&old_alternatives[..], &new_alternatives[..])
            && !is_kind(old_one)
            && !is_kind(new_one)
        {
            return self.shape(old_one, new_one, &at);
        }
        self.alternatives(old_alternatives, new_alternatives, &at);
    }

    /// Compares two choices by their alternatives: one gone is a kind
    /// removed, and one new a kind added.
    fn alternatives(&mut self, old: Vec<(String, Value)>, new: Vec<(String, Value)>, at: &str) {
        let (old_defs, new_defs) = (Rc::clone(&self.old_defs), Rc::clone(&self.new_defs));
        let old: Map<String, Value> = old.into_iter().collect();
        let new: Map<String, Value> = new.into_iter().collect();
        let (removed, added) = unmatched(&old, &new);
        self.removed_and_added(
            &removed,
            &added,
            |key| format!("{at}({key})"),
            |old, new| body_of(old, &old_defs) == body_of(new, &new_defs),
            |_| (ChangeKind::Added, ""),
        );
        for (key, old_alternative) in &old {
            let Some(new_alternative) = new.get(key) else {
                continue;
            };
            let at = format!("{at}({key})");
            if old_alternative.get("$ref").is_some() || new_alternative.get("$ref").is_some() {
                self.schema(old_alternative, new_alternative, &at);
            } else if old_alternative.get("const").is_none() {
                self.shape(old_alternative, new_alternative, &at);
            }
        }
    }

    /// Compares two schemas that are no choice.
    fn shape(&mut self, old: &Value, new: &Value, at: &str) {
        let (old_type, new_type) = (type_of(old), type_of(new));
        if old_type != new_type {
            let detail = format!("was {old_type}, is {new_type}");
            return self.change(ChangeKind::Retyped, at, detail);
        }
        match old_type.as_str() {
            "integer" | "number" | "string" => {
                let (old_format, new_format) = (format_of(old), format_of(new));
                if old_format != new_format {
                    let detail = format!("was {old_type} {old_format}, is {new_type} {new_format}");
                    self.change(ChangeKind::Retyped, at, detail);
                }
            }
            "array" => self.array(old, new, at),
            "object" => self.object(old, new, at),
            _ => {}
        }
    }

    fn array(&mut self, old: &Value, new: &Value, at: &str) {
        let tuple = |schema: &Value| schema.get("prefixItems").and_then(Value::as_array).cloned();
        match (tuple(old), tuple(new)) {
            (Some(old_items), Some(new_items)) if old_items.len() == new_items.len() => {
                for (index, (old_item, new_item)) in old_items.iter().zip(&new_items).enumerate() {
                    self.schema(old_item, new_item, &format!("{at}[{index}]"));
                }
            }
            (None, None) => {
                let length = |schema: &Value| {
                    (
                        schema.get("minItems").cloned(),
                        schema.get("maxItems").cloned(),
                    )
                };
                if length(old) != length(new) {
                    self.change(ChangeKind::Retyped, at, "holds another number of items");
                }
                let anything = Value::Bool(true);
                self.schema(
                    old.get("items").unwrap_or(&anything),
                    new.get("items").unwrap_or(&anything),
                    &format!("{at}[]"),
                );
            }
            _ => self.change(ChangeKind::Retyped, at, "holds other items"),
        }
    }

    fn object(&mut self, old: &Value, new: &Value, at: &str) {
        match (map_values(old), map_values(new)) {
            (Some(old_values), Some(new_values)) => {
                return self.schema(old_values, new_values, &format!("{at}.*"));
            }
            (None, None) => {}
            _ => return self.change(ChangeKind::Retyped, at, "was a map, or is one now"),
        }
        let (old_properties, new_properties) =
            (section_of(old, "properties"), section_of(new, "properties"));
        let (old_required, new_required) = (required(old), required(new));
        let (removed, added) = unmatched(&old_properties, &new_properties);
        self.removed_and_added(
            &removed,
            &added,
            |name| format!("{at}.{name}"),
            |old, new| without_default(old) == without_default(new),
            // A field added is required of an older writer, which never sent
            // it, unless it may be left out.
            |name| {
                if new_required.contains(name) {
                    (ChangeKind::NewlyRequired, "an older writer leaves it out")
                } else {
                    (ChangeKind::Added, "")
                }
            },
        );
        for (name, old_property) in &old_properties {
            let Some(new_property) = new_properties.get(name) else {
                continue;
            };
            let field = format!("{at}.{name}");
            let was_required = old_required.contains(name);
            let is_required = new_required.contains(name);
            let old_default = old_property.get("default");
            let new_default = new_property.get("default");
            if !was_required && is_required {
                if old_default.is_some() {
                    self.change(ChangeKind::DroppedDefault, &field, "is now required");
                } else {
                    self.change(
                        ChangeKind::NewlyRequired,
                        &field,
                        "may no longer be left out",
                    );
                }
            } else if let (Some(old_default), Some(new_default)) = (old_default, new_default)
                && old_default != new_default
            {
                let detail = format!("was {old_default}, is {new_default}");
                self.change(ChangeKind::DroppedDefault, &field, detail);
            }
            self.schema(old_property, new_property, &field);
        }
    }
}

/// `schema` with its references into `defs` followed, and the definition it
/// was last named by.
fn resolve<'a>(
    defs: &'a Map<String, Value>,
    mut schema: &'a Value,
) -> (&'a Value, Option<&'a str>) {
    let mut name = None;
    // A reference to a reference is followed a few steps at most.
    for _ in 0..8 {
        let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let defined = reference.strip_prefix("#/$defs/").unwrap_or(reference);
        let Some(definition) = defs.get(defined) else {
            break;
        };
        name = Some(defined);
        schema = definition;
    }
    (schema, name)
}

/// Named values, as found in one schema alone.
type Named<'a> = Vec<(&'a str, &'a Value)>;

/// The names in `old` alone and in `new` alone.
fn unmatched<'a>(
    old: &'a Map<String, Value>,
    new: &'a Map<String, Value>,
) -> (Named<'a>, Named<'a>) {
    let alone = |one: &'a Map<String, Value>, other: &'a Map<String, Value>| {
        one.iter()
            .filter(|(name, _)| !other.contains_key(*name))
            .map(|(name, value)| (name.as_str(), value))
            .collect()
    };
    (alone(old, new), alone(new, old))
}

fn required(schema: &Value) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// The alternatives `schema` admits, each by the key that tells it apart on the
/// wire. A schema that is no choice is one alternative.
fn alternatives(defs: &Map<String, Value>, schema: &Value) -> Vec<(String, Value)> {
    let mut alternatives = Vec::new();
    collect_alternatives(defs, schema, &mut alternatives, 0);
    let mut seen = BTreeMap::<String, usize>::new();
    alternatives
        .into_iter()
        .map(|(key, value)| {
            let count = seen.entry(key.clone()).or_default();
            *count += 1;
            match *count {
                1 => (key, value),
                count => (format!("{key}#{count}"), value),
            }
        })
        .collect()
}

fn collect_alternatives(
    defs: &Map<String, Value>,
    schema: &Value,
    into: &mut Vec<(String, Value)>,
    depth: usize,
) {
    if let Some(choices) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        && depth < 8
    {
        for choice in choices {
            let (resolved, _) = resolve(defs, choice);
            if is_choice(resolved) {
                collect_alternatives(defs, resolved, into, depth + 1);
            } else {
                into.push((key_of(resolved), choice.clone()));
            }
        }
        return;
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        for value in values {
            into.push((value.to_string(), json!({ "const": value })));
        }
        return;
    }
    into.push((key_of(schema), schema.clone()));
}

/// Whether `schema` may be null, and what it is otherwise.
fn without_null(schema: &Value) -> (bool, Value) {
    let is_null = |schema: &Value| schema.get("type").and_then(Value::as_str) == Some("null");
    if let Some(types) = schema.get("type").and_then(Value::as_array)
        && types.iter().any(|kind| kind == "null")
    {
        let rest: Vec<Value> = types
            .iter()
            .filter(|kind| *kind != "null")
            .cloned()
            .collect();
        let mut schema = schema.clone();
        schema["type"] = match &rest[..] {
            [one] => one.clone(),
            _ => Value::Array(rest),
        };
        return (true, schema);
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(choices) = schema.get(keyword).and_then(Value::as_array)
            && choices.iter().any(is_null)
        {
            let rest: Vec<Value> = choices
                .iter()
                .filter(|choice| !is_null(choice))
                .cloned()
                .collect();
            return match &rest[..] {
                [one] => (true, one.clone()),
                _ => (true, json!({ keyword: rest })),
            };
        }
    }
    (false, schema.clone())
}

fn is_choice(schema: &Value) -> bool {
    ["oneOf", "anyOf", "enum"]
        .iter()
        .any(|keyword| schema.get(keyword).is_some())
}

/// Whether an alternative is one kind among others, told apart on the wire by
/// a constant: a unit variant, or a variant by its tag.
fn is_kind(alternative: &Value) -> bool {
    alternative.get("const").is_some()
        || tag_of(alternative).is_some()
        || external_tag_of(alternative).is_some()
}

/// The property an internally tagged variant is told apart by, and its value.
fn tag_of(schema: &Value) -> Option<(String, Value)> {
    let properties = schema.get("properties")?.as_object()?;
    let required = required(schema);
    properties
        .iter()
        .filter(|(name, _)| required.contains(*name))
        .find_map(|(name, property)| Some((name.clone(), property.get("const")?.clone())))
}

/// The one property an externally tagged variant is wrapped in.
fn external_tag_of(schema: &Value) -> Option<String> {
    let properties = schema.get("properties")?.as_object()?;
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    let only = properties.keys().next()?;
    let wrapped =
        closed && properties.len() == 1 && required(schema).into_iter().eq([only.clone()]);
    wrapped.then(|| only.clone())
}

/// The key an alternative is told apart from the others by.
fn key_of(schema: &Value) -> String {
    if let Some(value) = schema.get("const") {
        return value.to_string();
    }
    if let Some((tag, value)) = tag_of(schema) {
        return format!("{tag}={value}");
    }
    if let Some(tag) = external_tag_of(schema) {
        return tag;
    }
    type_of(schema)
}

/// What an alternative holds beside what tells it apart, so two that hold the
/// same under different tags read as one renamed.
fn body_of(alternative: &Value, defs: &Map<String, Value>) -> Value {
    let (schema, _) = resolve(defs, alternative);
    if schema.get("const").is_some() {
        return Value::Null;
    }
    if let Some((tag, _)) = tag_of(schema) {
        let mut schema = schema.clone();
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove(&tag);
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|required| *required != tag);
        }
        return schema;
    }
    if let Some(tag) = external_tag_of(schema) {
        return schema["properties"][&tag].clone();
    }
    schema.clone()
}

fn type_of(schema: &Value) -> String {
    match schema {
        Value::Bool(true) => return "anything".to_owned(),
        Value::Bool(false) => return "nothing".to_owned(),
        _ => {}
    }
    match schema.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(kinds @ Value::Array(_)) => kinds.to_string(),
        _ if schema.get("properties").is_some() || schema.get("additionalProperties").is_some() => {
            "object".to_owned()
        }
        _ if schema.get("items").is_some() || schema.get("prefixItems").is_some() => {
            "array".to_owned()
        }
        _ if schema.get("const").is_some() => "constant".to_owned(),
        _ => "anything".to_owned(),
    }
}

fn format_of(schema: &Value) -> String {
    schema
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("unformatted")
        .to_owned()
}

/// The schema of a map's values, where `schema` is a map rather than an
/// object of named fields.
fn map_values(schema: &Value) -> Option<&Value> {
    let named = schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty());
    match schema.get("additionalProperties") {
        Some(Value::Bool(false)) | None => None,
        Some(values) if !named => Some(values),
        Some(_) => None,
    }
}

fn without_default(schema: &Value) -> Value {
    let mut schema = schema.clone();
    if let Some(schema) = schema.as_object_mut() {
        schema.remove("default");
    }
    schema
}
