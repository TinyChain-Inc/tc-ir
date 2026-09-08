use std::future::Future;
use std::pin::Pin;

use pathlink::PathSegment;
use tc_error::{TCError, TCResult};

use crate::{Map, Scalar, Transaction};

/// Native verbs supported by TinyChain routers and projected by adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Method {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "PUT" => Ok(Self::Put),
            "POST" => Ok(Self::Post),
            "DELETE" => Ok(Self::Delete),
            _ => Err(format!("unsupported method: {value}")),
        }
    }
}

/// The minimal native state capability required by routing.
pub trait StateInstance: Clone + Send + 'static {
    type Transaction: Transaction;
}

/// The erased future returned by one selected native verb.
pub type HandlerFuture<'a, T> = Pin<Box<dyn Future<Output = TCResult<T>> + Send + 'a>>;

pub type GetHandler<'a, 'txn, State> = Box<
    dyn FnOnce(&'txn <State as StateInstance>::Transaction, Scalar) -> HandlerFuture<'a, State>
        + Send
        + 'a,
>;
pub type PutHandler<'a, 'txn, State> = Box<
    dyn FnOnce(&'txn <State as StateInstance>::Transaction, Scalar, State) -> HandlerFuture<'a, ()>
        + Send
        + 'a,
>;
pub type PostHandler<'a, 'txn, State> = Box<
    dyn FnOnce(&'txn <State as StateInstance>::Transaction, Map<State>) -> HandlerFuture<'a, State>
        + Send
        + 'a,
>;
pub type DeleteHandler<'a, 'txn, State> = Box<
    dyn FnOnce(&'txn <State as StateInstance>::Transaction, Scalar) -> HandlerFuture<'a, ()>
        + Send
        + 'a,
>;

/// A routed native handler. Each implementation exposes only the verbs it owns.
pub trait Handler<'a, State: StateInstance>: Send {
    fn get<'txn>(self: Box<Self>) -> Option<GetHandler<'a, 'txn, State>>
    where
        'txn: 'a,
    {
        None
    }

    fn put<'txn>(self: Box<Self>) -> Option<PutHandler<'a, 'txn, State>>
    where
        'txn: 'a,
    {
        None
    }

    fn post<'txn>(self: Box<Self>) -> Option<PostHandler<'a, 'txn, State>>
    where
        'txn: 'a,
    {
        None
    }

    fn delete<'txn>(self: Box<Self>) -> Option<DeleteHandler<'a, 'txn, State>>
    where
        'txn: 'a,
    {
        None
    }
}

/// Resolve the most-specific handler mounted at a structural path.
pub trait Route<State: StateInstance>: Send + Sync {
    fn route<'a>(&'a self, path: &[PathSegment]) -> Option<Box<dyn Handler<'a, State> + 'a>>;
}

/// Uniform native method dispatch for every routed value.
pub trait Public<State>: Route<State>
where
    State: StateInstance,
{
    fn get<'a>(
        &'a self,
        txn: &'a State::Transaction,
        path: &[PathSegment],
        key: Scalar,
    ) -> impl Future<Output = TCResult<State>> + Send + 'a {
        let handler = self.route(path);
        let path = path_string(path);
        async move {
            let handler = handler.ok_or_else(|| TCError::not_found(path.clone()))?;
            let handler = handler
                .get()
                .ok_or_else(|| TCError::method_not_allowed(Method::Get, path))?;
            handler(txn, key).await
        }
    }

    fn put<'a>(
        &'a self,
        txn: &'a State::Transaction,
        path: &[PathSegment],
        key: Scalar,
        value: State,
    ) -> impl Future<Output = TCResult<()>> + Send + 'a {
        let handler = self.route(path);
        let path = path_string(path);
        async move {
            let handler = handler.ok_or_else(|| TCError::not_found(path.clone()))?;
            let handler = handler
                .put()
                .ok_or_else(|| TCError::method_not_allowed(Method::Put, path))?;
            handler(txn, key, value).await
        }
    }

    fn post<'a>(
        &'a self,
        txn: &'a State::Transaction,
        path: &[PathSegment],
        params: Map<State>,
    ) -> impl Future<Output = TCResult<State>> + Send + 'a {
        let handler = self.route(path);
        let path = path_string(path);
        async move {
            let handler = handler.ok_or_else(|| TCError::not_found(path.clone()))?;
            let handler = handler
                .post()
                .ok_or_else(|| TCError::method_not_allowed(Method::Post, path))?;
            handler(txn, params).await
        }
    }

    fn delete<'a>(
        &'a self,
        txn: &'a State::Transaction,
        path: &[PathSegment],
        key: Scalar,
    ) -> impl Future<Output = TCResult<()>> + Send + 'a {
        let handler = self.route(path);
        let path = path_string(path);
        async move {
            let handler = handler.ok_or_else(|| TCError::not_found(path.clone()))?;
            let handler = handler
                .delete()
                .ok_or_else(|| TCError::method_not_allowed(Method::Delete, path))?;
            handler(txn, key).await
        }
    }
}

impl<State, T> Public<State> for T
where
    State: StateInstance,
    T: Route<State>,
{
}

fn path_string(path: &[PathSegment]) -> String {
    let suffix = path
        .iter()
        .map(PathSegment::as_str)
        .collect::<Vec<_>>()
        .join("/");
    format!("/{suffix}")
}
