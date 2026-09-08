use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
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

/// One native method invocation, shared by inbound and nested dispatch.
pub enum MethodCall<State> {
    Get(Scalar),
    Put(Scalar, State),
    Post(Map<State>),
    Delete(Scalar),
}

impl<State> MethodCall<State> {
    pub fn method(&self) -> Method {
        match self {
            Self::Get(_) => Method::Get,
            Self::Put(_, _) => Method::Put,
            Self::Post(_) => Method::Post,
            Self::Delete(_) => Method::Delete,
        }
    }

    pub fn into_request(self) -> (Method, Option<State>)
    where
        State: From<Scalar> + From<Vec<State>> + From<Map<State>>,
    {
        match self {
            Self::Get(key) => (Method::Get, Some(State::from(key))),
            Self::Put(key, value) => (
                Method::Put,
                Some(State::from(vec![State::from(key), value])),
            ),
            Self::Post(params) => (Method::Post, Some(State::from(params))),
            Self::Delete(key) => (Method::Delete, Some(State::from(key))),
        }
    }

    pub async fn invoke(
        self,
        handler: &dyn Handler<State>,
        txn: &State::Transaction,
    ) -> TCResult<State>
    where
        State: StateInstance + Default,
    {
        match self {
            Self::Get(key) => handler.get(txn, key).await,
            Self::Put(key, value) => {
                handler.put(txn, key, value).await?;
                Ok(State::default())
            }
            Self::Post(params) => handler.post(txn, params).await,
            Self::Delete(key) => {
                handler.delete(txn, key).await?;
                Ok(State::default())
            }
        }
    }
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

/// IR analogue of `tc-transact`'s `Route` trait.
pub trait Route<State: StateInstance>: Send + Sync {
    /// Resolve the handler mounted at the given path.
    fn route(&self, path: &[PathSegment]) -> Option<Box<dyn Handler<State> + '_>>;
}

/// The minimal native state capability required by routing.
pub trait StateInstance: Clone + Send + 'static {
    type Transaction: Transaction;
}

/// A native route handler. Methods exchange state directly and know nothing
/// about views, serialization, or transport representations.
#[async_trait]
pub trait Handler<State>: Send + Sync
where
    State: StateInstance,
{
    async fn get(&self, _txn: &State::Transaction, _key: Scalar) -> TCResult<State> {
        Err(TCError::method_not_allowed(
            Method::Get,
            std::any::type_name::<Self>(),
        ))
    }

    async fn put(&self, _txn: &State::Transaction, _key: Scalar, _value: State) -> TCResult<()> {
        Err(TCError::method_not_allowed(
            Method::Put,
            std::any::type_name::<Self>(),
        ))
    }

    async fn post(&self, _txn: &State::Transaction, _params: Map<State>) -> TCResult<State> {
        Err(TCError::method_not_allowed(
            Method::Post,
            std::any::type_name::<Self>(),
        ))
    }

    async fn delete(&self, _txn: &State::Transaction, _key: Scalar) -> TCResult<()> {
        Err(TCError::method_not_allowed(
            Method::Delete,
            std::any::type_name::<Self>(),
        ))
    }
}

#[async_trait]
impl<State, H> Handler<State> for Arc<H>
where
    State: StateInstance,
    H: Handler<State> + ?Sized,
{
    async fn get(&self, txn: &State::Transaction, key: Scalar) -> TCResult<State> {
        (**self).get(txn, key).await
    }

    async fn put(&self, txn: &State::Transaction, key: Scalar, value: State) -> TCResult<()> {
        (**self).put(txn, key, value).await
    }

    async fn post(&self, txn: &State::Transaction, params: Map<State>) -> TCResult<State> {
        (**self).post(txn, params).await
    }

    async fn delete(&self, txn: &State::Transaction, key: Scalar) -> TCResult<()> {
        (**self).delete(txn, key).await
    }
}

#[async_trait]
impl<State, H> Handler<State> for &H
where
    State: StateInstance,
    H: Handler<State> + ?Sized,
{
    async fn get(&self, txn: &State::Transaction, key: Scalar) -> TCResult<State> {
        (**self).get(txn, key).await
    }

    async fn put(&self, txn: &State::Transaction, key: Scalar, value: State) -> TCResult<()> {
        (**self).put(txn, key, value).await
    }

    async fn post(&self, txn: &State::Transaction, params: Map<State>) -> TCResult<State> {
        (**self).post(txn, params).await
    }

    async fn delete(&self, txn: &State::Transaction, key: Scalar) -> TCResult<()> {
        (**self).delete(txn, key).await
    }
}

/// Uniform native method dispatch for every routed value.
pub trait Public<State>: Route<State>
where
    State: StateInstance,
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
