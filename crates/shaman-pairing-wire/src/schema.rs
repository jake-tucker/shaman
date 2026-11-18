//! The schema of a wire, as it is built from its types and written down: the
//! version and floor it was built at, the headers and standalone messages it
//! speaks, and each route a Peer may ask, by what it sends and is answered.
//!
//! A schema is JSON. Each payload is a JSON Schema (2020-12) of what a reader
//! of it requires — the `Deserialize` side of its type — and every type one
//! names is written once under `$defs`. Where a writer of an object may leave
//! out a field its reader requires, or always writes one its reader may do
//! without, `x-always-written` names the fields a writer always writes, from
//! the `Serialize` side of the type. An enum whose reader takes a kind it
//! does not know for a fallback says so by `x-unknown-kinds`. Descriptions
//! and titles are left out, so editing a type's documentation leaves its
//! schema as it was.

use std::collections::BTreeMap;

use schemars::{
    JsonSchema, SchemaGenerator,
    generate::{Contract, SchemaSettings},
};
use serde_json::{Map, Value, json};

/// A wire's schema, as it is built.
pub struct WireSchema {
    generator: Generators,
    version: u32,
    floor: u32,
    headers: BTreeMap<String, Value>,
    messages: BTreeMap<String, Value>,
    listener: BTreeMap<String, Route>,
    api: BTreeMap<String, Route>,
}

/// What one route takes and answers, each part absent where it has none.
#[derive(Default)]
struct Route {
    query: Option<Value>,
    body: Option<Part>,
    answer: Option<Part>,
}

/// A body: JSON of a type, or nothing where `or_nothing` holds; bytes of one
/// of some content types; an event stream of named events; or a stream of
/// JSON lines.
enum Part {
    Json { schema: Value, or_nothing: bool },
    Bytes(Vec<String>),
    Events(BTreeMap<String, Value>),
    Lines(Value),
}

/// How a header's value is spelled.
#[derive(Clone, Copy, Debug)]
pub enum HeaderEncoding {
    /// As text: a number, a word or an identity as its type writes it.
    Text,
    /// As the unpadded base64url of its JSON.
    Base64UrlJson,
}

impl HeaderEncoding {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Base64UrlJson => "base64url-json",
        }
    }
}

/// The two sides of every type: what its reader requires, and what its
/// writer writes.
struct Generators {
    reader: SchemaGenerator,
    writer: SchemaGenerator,
}

impl Generators {
    fn new() -> Self {
        let side = |contract| {
            let mut settings = SchemaSettings::draft2020_12();
            settings.contract = contract;
            settings.into_generator()
        };
        Self {
            reader: side(Contract::Deserialize),
            writer: side(Contract::Serialize),
        }
    }

    /// `T`'s reader's schema, saying where its writer writes otherwise.
    fn subschema<T: JsonSchema>(&mut self) -> Value {
        let mut reader = self.reader.subschema_for::<T>().to_value();
        let writer = self.writer.subschema_for::<T>().to_value();
        note_writer(&mut reader, &writer);
        reader
    }

    /// Every type named, each its reader's schema saying where its writer
    /// writes otherwise.
    fn definitions(&mut self) -> BTreeMap<String, Value> {
        let writer = self.writer.take_definitions(true);
        self.reader
            .take_definitions(true)
            .into_iter()
            .map(|(name, mut reader)| {
                if let Some(writer) = writer.get(&name) {
                    note_writer(&mut reader, writer);
                }
                (name, reader)
            })
            .collect()
    }
}

/// Notes in `reader`, beside each object whose writer writes otherwise than
/// its reader requires, the fields that writer always writes. The two
/// schemas are walked together where they have the same shape.
fn note_writer(reader: &mut Value, writer: &Value) {
    let (Value::Object(reader), Value::Object(writer)) = (reader, writer) else {
        return;
    };
    if reader.contains_key("properties") && writer.contains_key("properties") {
        let names = |schema: &Map<String, Value>| -> Vec<Value> {
            schema
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let (required, written) = (names(reader), names(writer));
        let sorted = |mut names: Vec<Value>| {
            names.sort_by(|one, other| one.as_str().cmp(&other.as_str()));
            names
        };
        if sorted(required.clone()) != sorted(written.clone()) {
            reader.insert("x-always-written".to_owned(), Value::Array(written));
        }
    }
    for (keyword, value) in reader.iter_mut() {
        let Some(other) = writer.get(keyword) else {
            continue;
        };
        match keyword.as_str() {
            "properties" => {
                if let (Value::Object(properties), Value::Object(others)) = (value, other) {
                    for (name, property) in properties.iter_mut() {
                        if let Some(other) = others.get(name) {
                            note_writer(property, other);
                        }
                    }
                }
            }
            "items" | "additionalProperties" => note_writer(value, other),
            "oneOf" | "anyOf" | "prefixItems" => {
                if let (Value::Array(choices), Value::Array(others)) = (value, other)
                    && choices.len() == others.len()
                {
                    for (choice, other) in choices.iter_mut().zip(others) {
                        note_writer(choice, other);
                    }
                }
            }
            _ => {}
        }
    }
}

impl WireSchema {
    /// A wire spoken at `version`, which speaks down to `floor`.
    pub fn new(version: u32, floor: u32) -> Self {
        Self {
            generator: Generators::new(),
            version,
            floor,
            headers: BTreeMap::new(),
            messages: BTreeMap::new(),
            listener: BTreeMap::new(),
            api: BTreeMap::new(),
        }
    }

    /// A header the wire speaks, whose value reads as `T` spelled by
    /// `encoding`.
    pub fn header<T: JsonSchema>(&mut self, name: &str, encoding: HeaderEncoding) -> &mut Self {
        let schema = json!({ "encoding": encoding.name(), "value": self.subschema::<T>() });
        assert!(
            self.headers.insert(name.to_owned(), schema).is_none(),
            "the header {name} is named twice"
        );
        self
    }

    /// A message the wire carries outside any one route, such as an Invite.
    pub fn message<T: JsonSchema>(&mut self, name: &str) -> &mut Self {
        let schema = self.subschema::<T>();
        assert!(
            self.messages.insert(name.to_owned(), schema).is_none(),
            "the message {name} is named twice"
        );
        self
    }

    /// A route of the Serving listener itself, such as the pairing ceremony's,
    /// written `METHOD /path`.
    pub fn listener(&mut self, route: &str) -> RouteSchema<'_> {
        Self::route(&mut self.generator, &mut self.listener, route)
    }

    /// A route of the Session API a Peer asks through its Pairing, written
    /// `METHOD /path` as the Session API spells it.
    pub fn api(&mut self, route: &str) -> RouteSchema<'_> {
        Self::route(&mut self.generator, &mut self.api, route)
    }

    fn route<'a>(
        generator: &'a mut Generators,
        routes: &'a mut BTreeMap<String, Route>,
        route: &str,
    ) -> RouteSchema<'a> {
        assert!(
            !routes.contains_key(route),
            "the route {route} is named twice"
        );
        RouteSchema {
            generator,
            route: routes.entry(route.to_owned()).or_default(),
        }
    }

    fn subschema<T: JsonSchema>(&mut self) -> Value {
        self.generator.subschema::<T>()
    }

    /// The schema, written down.
    pub fn document(mut self) -> Value {
        let definitions = self.generator.definitions();
        let mut document = Map::new();
        document.insert("protocol_version".to_owned(), json!(self.version));
        document.insert("protocol_floor".to_owned(), json!(self.floor));
        document.insert("headers".to_owned(), named(self.headers));
        document.insert("messages".to_owned(), named(self.messages));
        document.insert("listener".to_owned(), routes(self.listener));
        document.insert("api".to_owned(), routes(self.api));
        document.insert("$defs".to_owned(), named(definitions));
        let mut document = Value::Object(document);
        strip_annotations(&mut document, false);
        document
    }
}

/// One route of a [`WireSchema`] as it is described.
pub struct RouteSchema<'a> {
    generator: &'a mut Generators,
    route: &'a mut Route,
}

impl RouteSchema<'_> {
    /// The route reads its query string as `T`.
    pub fn query<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.query = Some(schema);
        self
    }

    /// The route takes a JSON body of `T`.
    pub fn body<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.body = Some(Part::Json {
            schema,
            or_nothing: false,
        });
        self
    }

    /// The route takes a body of bytes of one of `content_types`.
    pub fn bytes_body(self, content_types: &[&str]) -> Self {
        self.route.body = Some(bytes(content_types));
        self
    }

    /// The route answers JSON of `T`.
    pub fn answer<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.answer = Some(Part::Json {
            schema,
            or_nothing: false,
        });
        self
    }

    /// The route answers JSON of `T`, or nothing at all.
    pub fn answer_or_nothing<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.answer = Some(Part::Json {
            schema,
            or_nothing: true,
        });
        self
    }

    /// The route answers bytes of one of `content_types`.
    pub fn bytes_answer(self, content_types: &[&str]) -> Self {
        self.route.answer = Some(bytes(content_types));
        self
    }

    /// The route answers an event stream, which carries `name` events of `T`
    /// among the rest.
    pub fn event<T: JsonSchema>(mut self, name: &str) -> Self {
        let schema = self.subschema::<T>();
        let answer = self
            .route
            .answer
            .get_or_insert_with(|| Part::Events(BTreeMap::new()));
        let Part::Events(events) = answer else {
            panic!("a route that answers an event stream answers nothing else");
        };
        assert!(
            events.insert(name.to_owned(), schema).is_none(),
            "the event {name} is named twice"
        );
        self
    }

    /// The route answers a stream of lines, each JSON of `T`.
    pub fn lines<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.answer = Some(Part::Lines(schema));
        self
    }

    fn subschema<T: JsonSchema>(&mut self) -> Value {
        self.generator.subschema::<T>()
    }
}

fn bytes(content_types: &[&str]) -> Part {
    let mut content_types: Vec<String> =
        content_types.iter().map(|&kind| kind.to_owned()).collect();
    content_types.sort();
    Part::Bytes(content_types)
}

fn named(values: BTreeMap<String, Value>) -> Value {
    Value::Object(values.into_iter().collect())
}

fn routes(routes: BTreeMap<String, Route>) -> Value {
    Value::Object(
        routes
            .into_iter()
            .map(|(name, route)| {
                let mut parts = Map::new();
                if let Some(query) = route.query {
                    parts.insert("query".to_owned(), query);
                }
                if let Some(body) = route.body {
                    parts.insert("body".to_owned(), part(body));
                }
                if let Some(answer) = route.answer {
                    parts.insert("answer".to_owned(), part(answer));
                }
                (name, Value::Object(parts))
            })
            .collect(),
    )
}

fn part(part: Part) -> Value {
    match part {
        Part::Json {
            schema,
            or_nothing: false,
        } => json!({ "json": schema }),
        Part::Json {
            schema,
            or_nothing: true,
        } => json!({ "json": schema, "or_nothing": true }),
        Part::Bytes(content_types) => json!({ "bytes": content_types }),
        Part::Events(events) => json!({ "events": named(events) }),
        Part::Lines(schema) => json!({ "lines": schema }),
    }
}

/// The keywords that say what a schema is for rather than what it admits.
const ANNOTATIONS: [&str; 5] = ["description", "title", "examples", "$comment", "$schema"];

/// Takes every annotation out of `value`, whose keys are names (of
/// properties or definitions) where `names` holds rather than keywords.
fn strip_annotations(value: &mut Value, names: bool) {
    match value {
        Value::Object(map) => {
            if !names {
                for annotation in ANNOTATIONS {
                    map.remove(annotation);
                }
            }
            for (key, value) in map.iter_mut() {
                let holds_names = !names
                    && matches!(
                        key.as_str(),
                        "properties"
                            | "$defs"
                            | "patternProperties"
                            | "headers"
                            | "messages"
                            | "listener"
                            | "api"
                            | "events"
                    );
                // These hold values, not schemas: what they hold is left as
                // it is.
                if !names && matches!(key.as_str(), "default" | "const" | "enum") {
                    continue;
                }
                strip_annotations(value, holds_names);
            }
        }
        Value::Array(values) => {
            for value in values {
                strip_annotations(value, false);
            }
        }
        _ => {}
    }
}

/// `document` as it is written to a file: two-space indentation and a closing
/// newline.
pub fn written(document: &Value) -> String {
    let mut text = serde_json::to_string_pretty(document).expect("a schema always encodes");
    text.push('\n');
    text
}
