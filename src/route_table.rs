//! A router that keeps a list of the routes it serves as they are added, so
//! what a Server answers can be held, route by route, to what a Peer may ask
//! and to the Pairing wire's schema (ADR-0078).

use axum::{
    Router,
    handler::Handler,
    http::Method,
    routing::{MethodFilter, MethodRouter},
};

/// One route served: its method, `ANY` for one that answers every method,
/// and its path as the router spells it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Served {
    pub(crate) method: String,
    pub(crate) path: &'static str,
}

impl Served {
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

    /// These routes with their router passed through `layered`, such as to
    /// add a layer to every route so far.
    pub(crate) fn map_router(mut self, layered: impl FnOnce(Router<S>) -> Router<S>) -> Self {
        self.router = layered(self.router);
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
