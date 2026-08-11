use std::future::Future;

use pathlink::PathSegment;
use tc_error::{TCError, TCResult};

use crate::{Map, Scalar, Transaction};

/// Native verbs supported by TinyChain routers and projected by adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// IR analogue of `tc-transact`'s `Route` trait.
pub trait Route<State = ()>: Send + Sync {
    type Handler;

    /// Resolve the handler mounted at the given path.
    fn route(&self, path: &[PathSegment]) -> Option<Self::Handler>;
}

/// The minimal native state capability required by routing.
pub trait StateInstance: Clone + Send + 'static {
    type Transaction: Transaction;
}

/// A native route handler. Methods exchange state directly and know nothing
/// about views, serialization, or transport representations.
pub trait Handler<State>: Send + Sync
where
    State: StateInstance,
{
    fn get(
        &self,
        _txn: &State::Transaction,
        _key: Scalar,
    ) -> impl Future<Output = TCResult<State>> + Send {
        async {
            Err(TCError::method_not_allowed(
                Method::Get,
                std::any::type_name::<Self>(),
            ))
        }
    }

    fn put(
        &self,
        _txn: &State::Transaction,
        _key: Scalar,
        _value: State,
    ) -> impl Future<Output = TCResult<()>> + Send {
        async {
            Err(TCError::method_not_allowed(
                Method::Put,
                std::any::type_name::<Self>(),
            ))
        }
    }

    fn post(
        &self,
        _txn: &State::Transaction,
        _params: Map<State>,
    ) -> impl Future<Output = TCResult<State>> + Send {
        async {
            Err(TCError::method_not_allowed(
                Method::Post,
                std::any::type_name::<Self>(),
            ))
        }
    }

    fn delete(
        &self,
        _txn: &State::Transaction,
        _key: Scalar,
    ) -> impl Future<Output = TCResult<()>> + Send {
        async {
            Err(TCError::method_not_allowed(
                Method::Delete,
                std::any::type_name::<Self>(),
            ))
        }
    }
}

/// Uniform native method dispatch for every routed value.
pub trait Public<State>: Route<State>
where
    State: StateInstance,
    Self::Handler: Handler<State>,
{
    fn get(
        &self,
        txn: &State::Transaction,
        path: &[PathSegment],
        key: Scalar,
    ) -> impl Future<Output = TCResult<State>> + Send {
        async move {
            self.route(path)
                .ok_or_else(|| TCError::not_found(path_string(path)))?
                .get(txn, key)
                .await
        }
    }

    fn put(
        &self,
        txn: &State::Transaction,
        path: &[PathSegment],
        key: Scalar,
        value: State,
    ) -> impl Future<Output = TCResult<()>> + Send {
        async move {
            self.route(path)
                .ok_or_else(|| TCError::not_found(path_string(path)))?
                .put(txn, key, value)
                .await
        }
    }

    fn post(
        &self,
        txn: &State::Transaction,
        path: &[PathSegment],
        params: Map<State>,
    ) -> impl Future<Output = TCResult<State>> + Send {
        async move {
            self.route(path)
                .ok_or_else(|| TCError::not_found(path_string(path)))?
                .post(txn, params)
                .await
        }
    }

    fn delete(
        &self,
        txn: &State::Transaction,
        path: &[PathSegment],
        key: Scalar,
    ) -> impl Future<Output = TCResult<()>> + Send {
        async move {
            self.route(path)
                .ok_or_else(|| TCError::not_found(path_string(path)))?
                .delete(txn, key)
                .await
        }
    }
}

impl<State, T> Public<State> for T
where
    State: StateInstance,
    T: Route<State>,
    T::Handler: Handler<State>,
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
