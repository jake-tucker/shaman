//! A router that keeps a list of the routes it serves as they are added, so
//! what a Server answers can be held, route by route, to what a Peer may ask
//! and to the Pairing wire's schema (ADR-0078).

use std::convert::Infallible;

use axum::{
    Router,
    extract::Request,
    handler::Handler,
    http::Method,
    response::IntoResponse,
    routing::{MethodFilter, MethodRouter, Route},
};
use tower_layer::Layer;
use tower_service::Service;

/// One route served: its method, `ANY` for one that answers every method,
/// and its path as the router spells it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Served {
    pub(crate) method: String,
    pub(crate) path: &'static str,
}

impl Served {
    /// Whether this route answers `method` at `path`, a path as a request
    /// spells it: each `{name}` of the route's taking one segment, a closing
    /// `{*name}` the rest, and a `GET` route a `HEAD` too, as axum answers
    /// one.
    pub(crate) fn answers(&self, method: &Method, path: &str) -> bool {
        let method_matches = self.method == "ANY"
            || self.method == method.as_str()
            || (self.method == "GET" && *method == Method::HEAD);
        if !method_matches {
            return false;
        }
        let mut asked = path.trim_start_matches('/').split('/');
        for segment in self.path.trim_start_matches('/').split('/') {
            if segment.starts_with("{*") {
                return asked.next().is_some_and(|rest| !rest.is_empty());
            }
            match asked.next() {
                Some(given) if segment.starts_with('{') => {
                    if given.is_empty() {
                        return false;
                    }
                }
                Some(given) if given == segment => {}
                _ => return false,
            }
        }
        asked.next().is_none()
    }

    /// The route as the Pairing wire's schema names it: `METHOD /path`.
    #[cfg(test)]
    pub(crate) fn named(&self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

/// A [`Router`] with the list of the routes it serves.
pub(crate) struct Routes<S> {
    router: Router<S>,
    served: Vec<Served>,
}

impl<S: Clone + Send + Sync + 'static> Routes<S> {
    pub(crate) fn new() -> Self {
        Self {
            router: Router::new(),
            served: Vec::new(),
        }
    }

    pub(crate) fn get<H: Handler<T, S>, T: 'static>(self, path: &'static str, handler: H) -> Self {
        self.on(Method::GET, path, handler)
    }

    pub(crate) fn head<H: Handler<T, S>, T: 'static>(self, path: &'static str, handler: H) -> Self {
        self.on(Method::HEAD, path, handler)
    }

    pub(crate) fn post<H: Handler<T, S>, T: 'static>(self, path: &'static str, handler: H) -> Self {
        self.on(Method::POST, path, handler)
    }

    pub(crate) fn put<H: Handler<T, S>, T: 'static>(self, path: &'static str, handler: H) -> Self {
        self.on(Method::PUT, path, handler)
    }

    pub(crate) fn delete<H: Handler<T, S>, T: 'static>(
        self,
        path: &'static str,
        handler: H,
    ) -> Self {
        self.on(Method::DELETE, path, handler)
    }

    /// `path` answered by `handler` whatever its method.
    pub(crate) fn any<H: Handler<T, S>, T: 'static>(self, path: &'static str, handler: H) -> Self {
        self.with(
            "ANY".to_owned(),
            path,
            axum::routing::any(handler),
            |route| route,
        )
    }

    /// `path` answered for `method` by `handler`, its method router passed
    /// through `layered`, such as to bound the body it reads.
    pub(crate) fn on_layered<H: Handler<T, S>, T: 'static>(
        self,
        method: Method,
        path: &'static str,
        handler: H,
        layered: impl FnOnce(MethodRouter<S>) -> MethodRouter<S>,
    ) -> Self {
        let filter = MethodFilter::try_from(method.clone()).expect("a method a router answers");
        self.with(
            method.to_string(),
            path,
            axum::routing::on(filter, handler),
            layered,
        )
    }

    fn on<H: Handler<T, S>, T: 'static>(
        self,
        method: Method,
        path: &'static str,
        handler: H,
    ) -> Self {
        self.on_layered(method, path, handler, |route| route)
    }

    fn with(
        mut self,
        method: String,
        path: &'static str,
        route: MethodRouter<S>,
        layered: impl FnOnce(MethodRouter<S>) -> MethodRouter<S>,
    ) -> Self {
        self.router = self.router.route(path, layered(route));
        self.served.push(Served { method, path });
        self
    }

    /// These routes and `other`'s.
    pub(crate) fn merge(mut self, other: Self) -> Self {
        self.router = self.router.merge(other.router);
        self.served.extend(other.served);
        self
    }

    /// These routes, each behind `layer`, which can add none of its own.
    pub(crate) fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.router = self.router.route_layer(layer);
        self
    }

    pub(crate) fn into_parts(self) -> (Router<S>, Vec<Served>) {
        (self.router, self.served)
    }

    #[cfg(test)]
    pub(crate) fn served(self) -> Vec<Served> {
        self.served
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(method: &str, path: &'static str) -> Served {
        Served {
            method: method.to_owned(),
            path,
        }
    }

    #[test]
    fn a_route_answers_the_paths_its_template_spells() {
        let session = served("GET", "/v1/sessions/{session_id}/events");
        assert!(session.answers(&Method::GET, "/v1/sessions/abc/events"));
        assert!(session.answers(&Method::HEAD, "/v1/sessions/abc/events"));
        assert!(!session.answers(&Method::POST, "/v1/sessions/abc/events"));
        assert!(!session.answers(&Method::GET, "/v1/sessions//events"));
        assert!(!session.answers(&Method::GET, "/v1/sessions/abc/events/more"));
        assert!(!session.answers(&Method::GET, "/v1/sessions/abc"));

        let remote = served("ANY", "/v1/remotes/{name}/{*path}");
        assert!(remote.answers(&Method::DELETE, "/v1/remotes/laptop/v1/sessions"));
        assert!(!remote.answers(&Method::GET, "/v1/remotes/laptop"));
        assert!(!remote.answers(&Method::GET, "/v1/remotes/laptop/"));
    }
}
