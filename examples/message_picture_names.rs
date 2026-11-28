//! Reads a Message's Markdown as the Server would to find its pictures, for the 0822 probe.
//!
//! ADR-0086 has the Server read an agent's Message as CommonMark, with the parser the TUI and the
//! desktop app's Markdown view are built on, and take every image's destination. This reads each
//! capture `docs/validation/0822-message-pictures/probe/probe.py` kept and prints, for every
//! Message, each place a file could be named, as `pulldown-cmark` hands it over under the TUI's
//! options: an image's destination, a link's destination, a code span, a code block, and the plain
//! text between them.
//!
//! ```sh
//! cargo run --example message_picture_names -- docs/validation/0822-message-pictures/captures/**/*.json
//! ```
//!
//! One JSON line a capture: `{"file", "messages": [[{"kind", "value"}, …], …]}`, where `kind` is
//! `image`, `link`, `code`, `code_block`, `html` or `text`. `classify.py` beside the probe reads
//! these lines and says how each picture was named.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

/// The options `src/tui/markdown.rs` parses a Message under.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

fn places(markdown: &str) -> Vec<Value> {
    let mut places = Vec::new();
    let mut in_code_block = false;
    let mut text = String::new();
    let flush = |text: &mut String, places: &mut Vec<Value>, kind: &str| {
        if !text.is_empty() {
            places.push(json!({ "kind": kind, "value": std::mem::take(text) }));
        }
    };
    for event in Parser::new_ext(markdown, options()) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                flush(&mut text, &mut places, "text");
                places.push(json!({ "kind": "image", "value": dest_url.as_ref() }));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                flush(&mut text, &mut places, "text");
                places.push(json!({ "kind": "link", "value": dest_url.as_ref() }));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut text, &mut places, "text");
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut text, &mut places, "code_block");
                in_code_block = false;
            }
            Event::Code(code) => {
                flush(&mut text, &mut places, "text");
                places.push(json!({ "kind": "code", "value": code.as_ref() }));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                flush(&mut text, &mut places, "text");
                places.push(json!({ "kind": "html", "value": html.as_ref() }));
            }
            Event::Text(part) => text.push_str(&part),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::Heading(_)) if !in_code_block => {
                text.push('\n');
            }
            _ => {}
        }
    }
    flush(&mut text, &mut places, "text");
    places
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for file in std::env::args().skip(1) {
        let capture: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        let messages = capture["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| places(message["text"].as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        println!("{}", json!({ "file": file, "messages": messages }));
    }
    Ok(())
}
