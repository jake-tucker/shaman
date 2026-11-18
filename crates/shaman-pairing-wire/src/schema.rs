//! The schema of a wire, as it is built from its types and written down: the
//! version and floor it was built at, the headers and standalone messages it
//! speaks, and each route a Peer may ask, by what it sends and is answered.
//!
//! A schema is JSON. Each payload is a JSON Schema (2020-12) of what a reader
//! of it requires — the `Deserialize` side of its type — and every type one
//! names is written once under `$defs`. Descriptions and titles are left out,
//! so editing a type's documentation leaves its schema as it was.

use std::collections::BTreeMap;

use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde_json::{Map, Value, json};

/// A wire's schema, as it is built.
pub struct WireSchema {
    generator: SchemaGenerator,
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

/// A body: JSON of a type, bytes of a content type, an event stream of named
/// events, or a stream of JSON lines.
enum Part {
    Json(Value),
    Bytes(String),
    Events(BTreeMap<String, Value>),
    Lines(Value),
}

impl WireSchema {
    /// A wire spoken at `version`, which speaks down to `floor`.
    pub fn new(version: u32, floor: u32) -> Self {
        Self {
            generator: SchemaSettings::draft2020_12().into_generator(),
            version,
            floor,
            headers: BTreeMap::new(),
            messages: BTreeMap::new(),
            listener: BTreeMap::new(),
            api: BTreeMap::new(),
        }
    }

    /// A header the wire speaks, whose value reads as `T`.
    pub fn header<T: JsonSchema>(&mut self, name: &str) -> &mut Self {
        let schema = self.subschema::<T>();
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
        generator: &'a mut SchemaGenerator,
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
        self.generator.subschema_for::<T>().to_value()
    }

    /// The schema, written down.
    pub fn document(mut self) -> Value {
        let definitions: BTreeMap<String, Value> =
            self.generator.take_definitions(true).into_iter().collect();
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
    generator: &'a mut SchemaGenerator,
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
        self.route.body = Some(Part::Json(schema));
        self
    }

    /// The route takes a body of bytes of `content_type`.
    pub fn bytes_body(self, content_type: &str) -> Self {
        self.route.body = Some(Part::Bytes(content_type.to_owned()));
        self
    }

    /// The route answers JSON of `T`.
    pub fn answer<T: JsonSchema>(mut self) -> Self {
        let schema = self.subschema::<T>();
        self.route.answer = Some(Part::Json(schema));
        self
    }

    /// The route answers bytes of whatever content type they were stored as.
    pub fn bytes_answer(self) -> Self {
        self.route.answer = Some(Part::Bytes("*/*".to_owned()));
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
        self.generator.subschema_for::<T>().to_value()
    }
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
        Part::Json(schema) => json!({ "json": schema }),
        Part::Bytes(content_type) => json!({ "bytes": content_type }),
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
