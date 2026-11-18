//! Judging a wire's schema against the one it changes: what an older side
//! could not survive, and whether the version and floor moved as they must
//! (ADR-0078).
//!
//! Types are compared by what they look like on the wire, never by their Rust
//! names, so renaming a type changes nothing here. Each change is found where
//! a payload leads to it, from a route, header or message, and a type reached
//! from several places is compared once. Both sides of a type are read: what
//! its reader requires, and what its writer always writes. Any difference in
//! a keyword this check does not otherwise compare is reported as a break,
//! and a keyword it does not know at all refuses the schema, so nothing
//! passes unread.

use std::{
    borrow::Cow,
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
    /// A writer may leave out what an older reader requires.
    LeftOut,
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
            Self::LeftOut => "may be left out",
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
    /// Each enum an answer carries that takes no fallback for a kind its
    /// reader does not know, so an older reader could not read a new kind.
    pub intolerant: Vec<String>,
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
        problems.extend(intolerance(&self.intolerant));
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

/// What is wrong of `intolerant`, the enums an answer carries with no
/// fallback for a kind they do not know.
pub fn intolerance(intolerant: &[String]) -> Option<String> {
    (!intolerant.is_empty()).then(|| {
        format!(
            "an answer carries {}, which an older reader could not read a new kind of: give \
             each a fallback for a kind it does not know, and mark it \
             `x-unknown-kinds = \"tolerated\"`",
            intolerant.join(", ")
        )
    })
}

/// The marker an enum whose reader takes a kind it does not know for a
/// fallback carries, as [`TOLERATED`], and a list whose reader passes over an
/// item of such a kind, as [`PASSED_OVER`].
pub const TOLERATES_UNKNOWN_KINDS: &str = "x-unknown-kinds";

/// What [`TOLERATES_UNKNOWN_KINDS`] says of an enum that takes a fallback.
pub const TOLERATED: &str = "tolerated";

/// What [`TOLERATES_UNKNOWN_KINDS`] says of a list that passes over an item
/// of a kind its reader does not know.
pub const PASSED_OVER: &str = "passed-over";

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
    comparer.headers(
        &section_of(base, "headers"),
        &section_of(current, "headers"),
    );
    comparer.named(
        &section_of(base, "messages"),
        &section_of(current, "messages"),
        &|name| format!("message {name}"),
        true,
    );
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
        intolerant: intolerant_enums(current),
    })
}

/// The keywords a schema's object may carry that this check reads.
const READ: [&str; 22] = [
    "$ref",
    "additionalProperties",
    "anyOf",
    "const",
    "default",
    "enum",
    "format",
    "items",
    "maxItems",
    "maximum",
    "maxLength",
    "minItems",
    "minimum",
    "minLength",
    "oneOf",
    "pattern",
    "prefixItems",
    "properties",
    "required",
    "type",
    "x-always-written",
    TOLERATES_UNKNOWN_KINDS,
];

/// Every schema `document` holds at its top, each by the side that reads it:
/// a definition, or what a request or an answer carries.
fn payloads(document: &Value) -> Vec<(&'static str, &Value)> {
    let values = |section: &str| {
        document
            .get(section)
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(Map::values)
            .collect::<Vec<_>>()
    };
    let mut payloads: Vec<(&'static str, &Value)> = Vec::new();
    for definition in values("$defs") {
        payloads.push(("definition", definition));
    }
    for header in values("headers") {
        let side = if header_sides(header).contains("answer") {
            "answer"
        } else {
            "request"
        };
        payloads.extend(header.get("value").map(|value| (side, value)));
    }
    for message in values("messages") {
        payloads.push(("answer", message));
    }
    for section in ["listener", "api"] {
        for route in values(section) {
            payloads.extend(route.get("query").map(|query| ("request", query)));
            for (part, side) in [("body", "request"), ("answer", "answer")] {
                let Some(part) = route.get(part) else {
                    continue;
                };
                payloads.extend(part.get("json").map(|json| (side, json)));
                payloads.extend(part.get("lines").map(|lines| (side, lines)));
                for event in part
                    .get("events")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(Map::values)
                {
                    payloads.push((side, event));
                }
            }
        }
    }
    payloads
}

/// The first keyword anywhere in `document`'s schemas that this check does
/// not read, so a schema that came to use one fails rather than passes
/// unread.
fn unread_keyword(document: &Value) -> Option<String> {
    fn schema(value: &Value) -> Option<String> {
        let Value::Object(schema_map) = value else {
            return None;
        };
        for (keyword, value) in schema_map {
            if !READ.contains(&keyword.as_str()) {
                return Some(keyword.clone());
            }
            let found = match keyword.as_str() {
                "properties" => value.as_object()?.values().find_map(schema),
                "items" | "additionalProperties" => schema(value),
                "anyOf" | "oneOf" | "prefixItems" => value.as_array()?.iter().find_map(schema),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    payloads(document)
        .into_iter()
        .find_map(|(_, payload)| schema(payload))
}

/// Each enum an answer of `document` carries, by its definition's name or
/// the type it lies within, that takes no fallback for a kind its reader does
/// not know.
fn intolerant_enums(document: &Value) -> Vec<String> {
    let defs = section_of(document, "$defs");
    let mut seen = HashSet::new();
    let mut intolerant = BTreeSet::new();
    // Each schema waiting, where it was reached, and whether a list holding
    // it passes over an item of a kind its reader does not know.
    let mut waiting: Vec<(String, Value, bool)> = payloads(document)
        .into_iter()
        .filter(|(side, _)| *side == "answer")
        .map(|(_, payload)| ("an answer".to_owned(), payload.clone(), false))
        .collect();
    while let Some((at, schema, passed_over)) = waiting.pop() {
        let (resolved, name) = resolve(&defs, &schema);
        if let Some(name) = name
            && !seen.insert(name.to_owned())
        {
            continue;
        }
        let at = name.map_or(at, str::to_owned);
        let (_, schema) = without_null(&resolved);
        let marker = schema
            .get(TOLERATES_UNKNOWN_KINDS)
            .or_else(|| resolved.get(TOLERATES_UNKNOWN_KINDS))
            .and_then(Value::as_str);
        let choices = alternatives(&defs, &schema);
        let kinds = choices.iter().filter(|(_, choice)| is_kind(choice)).count();
        if kinds > 0 && marker != Some(TOLERATED) && !passed_over {
            intolerant.insert(at.clone());
        }
        // A list marked so passes over an item of a kind it does not know,
        // so its items need no fallback of their own.
        let passes_over = marker == Some(PASSED_OVER);
        for (_, choice) in choices {
            // A variant that is a reference is followed to what it names
            // alone; the tag beside it is the enum's, already judged.
            if let Some(reference) = choice.get("$ref") {
                waiting.push((at.clone(), json!({ "$ref": reference }), false));
                continue;
            }
            // A variant's tag is what tells it apart, not a value of its own.
            let tag = tag_of(&choice).map(|(tag, _)| tag);
            for (name, property) in section_of(&choice, "properties") {
                if Some(&name) != tag.as_ref() {
                    waiting.push((at.clone(), property, false));
                }
            }
            if let Some(items) = choice.get("items")
                && items.is_object()
            {
                waiting.push((at.clone(), items.clone(), passes_over));
            }
            if let Some(values) = choice.get("additionalProperties")
                && values.is_object()
            {
                waiting.push((at.clone(), values.clone(), false));
            }
            for inner in choice
                .get("prefixItems")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                waiting.push((at.clone(), inner.clone(), false));
            }
        }
    }
    intolerant.into_iter().collect()
}

/// A kind no build of Shaman names, which samples of a newer side's kinds
/// are spelled with.
pub const INVENTED_KIND: &str = "a_kind_no_build_knows";

/// Samples of what a newer side could send of the definition `name` in
/// `document`, each of a kind this build does not know, in every shape the
/// definition tells its kinds apart by: a name alone, a tag, or the one key of
/// an object. A list marked [`PASSED_OVER`] is sampled as a list of such
/// items. None where the definition has no kinds.
pub fn invented_kinds(document: &Value, name: &str) -> Vec<Value> {
    let defs = section_of(document, "$defs");
    let Some(definition) = defs.get(name) else {
        return Vec::new();
    };
    if definition
        .get(TOLERATES_UNKNOWN_KINDS)
        .and_then(Value::as_str)
        == Some(PASSED_OVER)
    {
        return definition
            .get("items")
            .map(|items| kinds_of(&defs, items))
            .unwrap_or_default()
            .into_iter()
            .map(|item| json!([item]))
            .collect();
    }
    kinds_of(&defs, definition)
}

fn kinds_of(defs: &Map<String, Value>, schema: &Value) -> Vec<Value> {
    let (schema, _) = resolve(defs, schema);
    let (_, schema) = without_null(&schema);
    let alternatives: Vec<Value> = alternatives(defs, &schema)
        .into_iter()
        .map(|(_, alternative)| resolve(defs, &alternative).0.into_owned())
        .collect();
    // What every variant carries, a newer side's kind would carry too.
    let shared: Vec<(String, Value)> = alternatives
        .iter()
        .filter(|alternative| alternative.get("const").is_none())
        .map(|alternative| {
            let tag = tag_of(alternative).map(|(tag, _)| tag);
            section_of(alternative, "properties")
                .into_iter()
                .filter(|(name, _)| {
                    required(alternative).contains(name) && Some(name) != tag.as_ref()
                })
                .collect::<Vec<_>>()
        })
        .reduce(|shared, next| {
            shared
                .into_iter()
                .filter(|(name, _)| next.iter().any(|(other, _)| other == name))
                .collect()
        })
        .unwrap_or_default();
    let mut samples = Vec::new();
    for alternative in &alternatives {
        let sample = if alternative.get("const").is_some() {
            json!(INVENTED_KIND)
        } else if let Some((tag, _)) = tag_of(alternative) {
            let mut sample = json!({ tag: INVENTED_KIND, "held": { "anything": true } });
            for (name, property) in &shared {
                sample[name] = example(defs, property, 0);
            }
            sample
        } else if external_tag_of(alternative).is_some() {
            json!({ INVENTED_KIND: { "anything": true } })
        } else {
            continue;
        };
        if !samples.contains(&sample) {
            samples.push(sample);
        }
    }
    samples
}

/// A plain value `schema` takes: the least of its type, a nil identity for a
/// UUID, and an object of its required fields.
fn example(defs: &Map<String, Value>, schema: &Value, depth: usize) -> Value {
    let (schema, _) = resolve(defs, schema);
    let (_, schema) = without_null(&schema);
    if depth > 8 {
        return Value::Null;
    }
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some((_, first)) = alternatives(defs, &schema).into_iter().next()
        && is_choice(&schema)
    {
        return example(defs, &first, depth + 1);
    }
    match type_of(&schema).as_str() {
        "string" if format_of(&schema) == "uuid" => json!("00000000-0000-0000-0000-000000000000"),
        "string" => json!("x"),
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        "array" => json!([]),
        "object" => Value::Object(
            section_of(&schema, "properties")
                .into_iter()
                .filter(|(name, _)| required(&schema).contains(name))
                .map(|(name, property)| {
                    let value = example(defs, &property, depth + 1);
                    (name, value)
                })
                .collect(),
        ),
        _ => Value::Null,
    }
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

    fn headers(&mut self, old: &Map<String, Value>, new: &Map<String, Value>) {
        let label = |name: &str| format!("header {name}");
        let (removed, added) = unmatched(old, new);
        self.removed_and_added(
            &removed,
            &added,
            label,
            |old, new| old == new,
            // A header newly required is one an older side never sends.
            |name| {
                if header_required(&new[name]) {
                    (ChangeKind::NewlyRequired, "an older side sends none")
                } else {
                    (ChangeKind::Added, "")
                }
            },
            true,
        );
        for (name, old_header) in old {
            let Some(new_header) = new.get(name) else {
                continue;
            };
            let at = label(name);
            self.via = at.clone();
            let (old_encoding, new_encoding) = (&old_header["encoding"], &new_header["encoding"]);
            if old_encoding != new_encoding {
                let detail = format!("was spelled {old_encoding}, is {new_encoding}");
                self.change(ChangeKind::Retyped, &at, detail);
            }
            let (old_sides, new_sides) = (header_sides(old_header), header_sides(new_header));
            for side in old_sides.difference(&new_sides) {
                self.change(
                    ChangeKind::Removed,
                    &at,
                    format!("no longer sent with {side}s"),
                );
            }
            let required = header_required(new_header);
            for side in new_sides.difference(&old_sides) {
                let kind = if required {
                    ChangeKind::NewlyRequired
                } else {
                    ChangeKind::Added
                };
                self.change(kind, &at, format!("now sent with {side}s"));
            }
            if !header_required(old_header) && required {
                self.change(
                    ChangeKind::NewlyRequired,
                    &at,
                    "an older side may leave it out",
                );
            }
            self.schema(&old_header["value"], &new_header["value"], &at);
        }
    }

    /// Compares two maps of named schemas, such as messages or events, the
    /// top of the wire's where `top` holds.
    fn named(
        &mut self,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
        label: &dyn Fn(&str) -> String,
        top: bool,
    ) {
        let (removed, added) = unmatched(old, new);
        self.removed_and_added(
            &removed,
            &added,
            label,
            |old, new| old == new,
            |_| (ChangeKind::Added, ""),
            top,
        );
        let via = self.via.clone();
        for (name, old_schema) in old {
            if let Some(new_schema) = new.get(name) {
                let at = label(name);
                self.via = if top { at.clone() } else { via.clone() };
                self.schema(old_schema, new_schema, &at);
            }
        }
        self.via = via;
    }

    fn routes(&mut self, old: &Map<String, Value>, new: &Map<String, Value>, prefix: &str) {
        let (removed, added) = unmatched(old, new);
        self.removed_and_added(
            &removed,
            &added,
            |route| format!("{prefix}{route}"),
            |old, new| old == new,
            |_| (ChangeKind::Added, ""),
            true,
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
            ["json", "bytes", "events", "lines"]
                .into_iter()
                .find(|kind| part.get(kind).is_some())
                .unwrap_or_default()
        };
        let (old_kind, new_kind) = (kind(old), kind(new));
        if old_kind != new_kind {
            let detail = format!("was {old_kind}, is {new_kind}");
            return self.change(ChangeKind::Retyped, at, detail);
        }
        let or_nothing = |part: &Value| part.get("or_nothing") == Some(&Value::Bool(true));
        match (or_nothing(old), or_nothing(new)) {
            (false, true) => self.change(ChangeKind::LeftOut, at, "may now be nothing at all"),
            (true, false) => self.change(ChangeKind::Added, at, "is now always given"),
            _ => {}
        }
        let (old, new) = (&old[old_kind], &new[new_kind]);
        match old_kind {
            "bytes" => {
                let types = |part: &Value| -> Map<String, Value> {
                    part.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|kind| (kind.to_owned(), Value::Null))
                        .collect()
                };
                let (old_types, new_types) = (types(old), types(new));
                let (removed, added) = unmatched(&old_types, &new_types);
                self.removed_and_added(
                    &removed,
                    &added,
                    |kind| format!("{at} of {kind}"),
                    |_, _| false,
                    |_| (ChangeKind::Added, ""),
                    false,
                );
            }
            "events" => {
                let empty = Map::new();
                let (old, new) = (
                    old.as_object().unwrap_or(&empty),
                    new.as_object().unwrap_or(&empty),
                );
                let route = at.to_owned();
                self.named(old, new, &|event| format!("{route} event {event}"), false);
            }
            _ => self.schema(old, new, at),
        }
    }

    /// Reports what is in `old` alone and `new` alone: one of each as a rename
    /// where `renamed` says their bodies match, and each added by the kind
    /// `added_kind` gives its name. Each is reached by itself where it stands
    /// at the `top` of the wire.
    fn removed_and_added(
        &mut self,
        removed: &[(&str, &Value)],
        added: &[(&str, &Value)],
        label: impl Fn(&str) -> String,
        renamed: impl Fn(&Value, &Value) -> bool,
        added_kind: impl Fn(&str) -> (ChangeKind, &'static str),
        top: bool,
    ) {
        let report = |comparer: &mut Self, kind, at: String, detail: String| {
            if top {
                comparer.via = at.clone();
            }
            comparer.change(kind, &at, detail);
        };
        if let ([(old_name, old_body)], [(new_name, new_body)]) = (removed, added)
            && renamed(old_body, new_body)
        {
            let detail = format!("now {}", label(new_name));
            return report(self, ChangeKind::Renamed, label(old_name), detail);
        }
        for (name, _) in removed {
            report(self, ChangeKind::Removed, label(name), String::new());
        }
        for (name, _) in added {
            let (kind, detail) = added_kind(name);
            report(self, kind, label(name), detail.to_owned());
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
        let (old_null, old) = without_null(&old);
        let (new_null, new) = without_null(&new);
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
        let marked = |schema: &Value| schema.get(TOLERATES_UNKNOWN_KINDS).cloned();
        if marked(&old) != marked(&new) {
            let (kind, detail) = if marked(&new).is_some() {
                (ChangeKind::Added, "now reads a kind it does not know")
            } else {
                (
                    ChangeKind::Retyped,
                    "no longer reads a kind it does not know",
                )
            };
            self.change(kind, &at, detail);
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
            false,
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
        self.unread(old, new, at);
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

    /// Reports as a break every difference between `old` and `new` in a
    /// keyword nothing else here compares, such as a bound.
    fn unread(&mut self, old: &Value, new: &Value, at: &str) {
        const COMPARED: [&str; 17] = [
            "$ref",
            "additionalProperties",
            "anyOf",
            "const",
            "default",
            "enum",
            "format",
            "items",
            "maxItems",
            "minItems",
            "oneOf",
            "prefixItems",
            "properties",
            "required",
            "type",
            "x-always-written",
            TOLERATES_UNKNOWN_KINDS,
        ];
        let keywords: BTreeSet<&String> = [old, new]
            .into_iter()
            .filter_map(Value::as_object)
            .flat_map(Map::keys)
            .filter(|keyword| !COMPARED.contains(&keyword.as_str()))
            .collect();
        for keyword in keywords {
            let (old_value, new_value) = (old.get(keyword), new.get(keyword));
            if old_value != new_value {
                let shown =
                    |value: Option<&Value>| value.map_or("absent".to_owned(), Value::to_string);
                let detail = format!(
                    "its {keyword} was {}, is {}",
                    shown(old_value),
                    shown(new_value)
                );
                self.change(ChangeKind::Retyped, at, detail);
            }
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
        let closed =
            |schema: &Value| schema.get("additionalProperties") == Some(&Value::Bool(false));
        match (closed(old), closed(new)) {
            (false, true) => {
                self.change(ChangeKind::Retyped, at, "refuses fields it does not know");
            }
            (true, false) => self.change(ChangeKind::Added, at, "takes fields it does not know"),
            _ => {}
        }
        let (old_properties, new_properties) =
            (section_of(old, "properties"), section_of(new, "properties"));
        let (old_required, new_required) = (required(old), required(new));
        let (old_written, new_written) = (always_written(old), always_written(new));
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
            false,
        );
        for (name, old_property) in &old_properties {
            let Some(new_property) = new_properties.get(name) else {
                continue;
            };
            let field = format!("{at}.{name}");
            let was_required = old_required.contains(name);
            let is_required = new_required.contains(name);
            if !was_required && is_required {
                if old_property.get("default").is_some() {
                    self.change(ChangeKind::DroppedDefault, &field, "is now required");
                } else {
                    self.change(
                        ChangeKind::NewlyRequired,
                        &field,
                        "may no longer be left out",
                    );
                }
            } else if !was_required
                && !is_required
                && let Some(detail) = default_changed(old_property, new_property)
            {
                self.change(ChangeKind::DroppedDefault, &field, detail);
            }
            if was_required && old_written.contains(name) && !new_written.contains(name) {
                self.change(
                    ChangeKind::LeftOut,
                    &field,
                    "a writer may now leave out what an older reader requires",
                );
            }
            self.schema(old_property, new_property, &field);
        }
    }
}

/// How the value a reader takes for a field left out changed between `old`
/// and `new`, where it did.
///
/// A default its writer would leave out goes unrecorded, and is taken to be
/// the empty value of its type — `false`, `0`, an empty string, list or map,
/// or null — as a writer skipping a field skips that value.
fn default_changed(old: &Value, new: &Value) -> Option<String> {
    let taken = |property: &Value| -> Option<Value> {
        property
            .get("default")
            .cloned()
            .or_else(|| without_null(property).0.then_some(Value::Null))
    };
    let empty = |value: &Value| match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
    };
    let shown = |value: &Option<Value>| {
        value.as_ref().map_or(
            "the value its writer leaves out".to_owned(),
            Value::to_string,
        )
    };
    let (old_taken, new_taken) = (taken(old), taken(new));
    let differs = match (&old_taken, &new_taken) {
        (Some(old), Some(new)) => old != new,
        (None, Some(known)) | (Some(known), None) => !empty(known),
        (None, None) => false,
    };
    differs.then(|| {
        format!(
            "a field left out now reads as {}, where it read as {}",
            shown(&new_taken),
            shown(&old_taken)
        )
    })
}

/// What a header is sent with: requests, answers, or both.
fn header_sides(header: &Value) -> BTreeSet<String> {
    names(header, "sent")
}

/// Whether a reader requires a header, where it says.
fn header_required(header: &Value) -> bool {
    header.get("required") == Some(&Value::Bool(true))
}

/// `schema` with its references into `defs` followed, and the definition it
/// was last named by. What a reference stands beside, such as an enum
/// variant's tag, is taken in with what it names.
fn resolve<'a>(
    defs: &'a Map<String, Value>,
    schema: &'a Value,
) -> (Cow<'a, Value>, Option<&'a str>) {
    let mut name = None;
    let mut resolved = Cow::Borrowed(schema);
    // A reference to a reference is followed a few steps at most.
    for _ in 0..8 {
        let Some(reference) = resolved.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let defined = reference.strip_prefix("#/$defs/").unwrap_or(reference);
        let Some((defined, definition)) = defs.get_key_value(defined) else {
            break;
        };
        name = Some(defined.as_str());
        let mut beside = resolved.as_object().cloned().unwrap_or_default();
        beside.remove("$ref");
        // A field's default belongs to the field, not to what it holds.
        beside.remove("default");
        resolved = if beside.is_empty() {
            Cow::Borrowed(definition)
        } else {
            Cow::Owned(merged(definition, &beside))
        };
    }
    (resolved, name)
}

/// `schema` with `beside` taken into it: their fields, required fields and
/// fields always written joined, and any other keyword of `beside` kept.
fn merged(schema: &Value, beside: &Map<String, Value>) -> Value {
    let mut merged = schema.as_object().cloned().unwrap_or_default();
    for (keyword, value) in beside {
        match (keyword.as_str(), merged.get_mut(keyword)) {
            ("properties", Some(Value::Object(properties))) => {
                for (name, property) in value.as_object().into_iter().flatten() {
                    properties.insert(name.clone(), property.clone());
                }
            }
            ("required" | "x-always-written", Some(Value::Array(names))) => {
                for name in value.as_array().into_iter().flatten() {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
            (_, Some(_)) => {}
            (_, None) => {
                merged.insert(keyword.clone(), value.clone());
            }
        }
    }
    Value::Object(merged)
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
    names(schema, "required")
}

/// The fields a writer of `schema` always writes: those its reader requires,
/// unless it says otherwise.
fn always_written(schema: &Value) -> BTreeSet<String> {
    if schema.get("x-always-written").is_some() {
        names(schema, "x-always-written")
    } else {
        required(schema)
    }
}

fn names(schema: &Value, keyword: &str) -> BTreeSet<String> {
    schema
        .get(keyword)
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
        // What stands beside the choice, such as the fields of a struct an
        // enum is flattened into, belongs to each alternative.
        let mut beside = schema.as_object().cloned().unwrap_or_default();
        for keyword in ["oneOf", "anyOf", TOLERATES_UNKNOWN_KINDS] {
            beside.remove(keyword);
        }
        for choice in choices {
            let choice = if beside.is_empty() {
                Cow::Borrowed(choice)
            } else {
                Cow::Owned(merged(choice, &beside))
            };
            let (resolved, _) = resolve(defs, &choice);
            if is_choice(&resolved) {
                collect_alternatives(defs, &resolved, into, depth + 1);
            } else {
                into.push((key_of(&resolved), choice.into_owned()));
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
            let mut beside = schema.as_object().cloned().unwrap_or_default();
            beside.remove(keyword);
            beside.remove("default");
            let rest = match &rest[..] {
                [one] => one.clone(),
                _ => json!({ keyword: rest }),
            };
            let schema = if beside.is_empty() {
                rest
            } else {
                merged(&rest, &beside)
            };
            return (true, schema);
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
    if let Some((tag, _)) = tag_of(&schema) {
        let mut schema = schema.into_owned();
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove(&tag);
        }
        for keyword in ["required", "x-always-written"] {
            if let Some(names) = schema.get_mut(keyword).and_then(Value::as_array_mut) {
                names.retain(|name| *name != tag);
            }
        }
        return schema;
    }
    if let Some(tag) = external_tag_of(&schema) {
        return schema["properties"][&tag].clone();
    }
    schema.into_owned()
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
