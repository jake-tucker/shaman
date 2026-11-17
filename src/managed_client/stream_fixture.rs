//! An event stream's answer built from its events alone, for a test of what a
//! reader makes of what a Server sends without a Server to send it.

/// An answer whose body is `events` as a Server streams them: each under its
/// name, with its id where it has one, and its data as given, so a test can
/// send what no Server of this build would.
pub(super) fn stream_of(events: &[(&str, Option<u64>, String)]) -> reqwest::Response {
    let body: String = events
        .iter()
        .map(|(name, id, data)| {
            let id = id.map(|id| format!("id: {id}\n")).unwrap_or_default();
            format!("event: {name}\n{id}data: {data}\n\n")
        })
        .collect();
    reqwest::Response::from(axum::http::Response::new(body))
}
