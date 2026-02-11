use std::future::Future;

use destream::de;
use pathlink::PathSegment;
use tc_error::{TCError, TCResult};

use crate::Transaction;

/// HTTP-like verbs supported by TinyChain routers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
}

/// IR analogue of `tc-transact`'s `Route` trait.
pub trait Route {
    type Handler;

    /// Resolve the handler mounted at the given path.
    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a Self::Handler>;
}

/// Marker trait implemented by every TinyChain handler.
pub trait Handler<T>: Send + Sync
where
    T: Transaction + ?Sized,
{
    fn method_not_supported(method: Method) -> TCError {
        TCError::method_not_allowed(method, std::any::type_name::<Self>())
    }
}

impl<T, H> Handler<T> for H
where
    T: Transaction + ?Sized,
    H: Send + Sync,
{
}

#[cfg(feature = "pyo3-conversions")]
pub trait FromPyRequest<'py>: Sized {
    type PyError;

    fn from_py(obj: &pyo3::Bound<'py, pyo3::PyAny>) -> Result<Self, Self::PyError>;
}

macro_rules! define_verb_handler {
    ($trait_name:ident, $fn_name:ident, $method:expr) => {
        pub trait $trait_name<T>: Handler<T>
        where
            T: Transaction + ?Sized,
        {
            type Request: de::FromStream<Context = Self::RequestContext>;
            type RequestContext: Send;
            type Response;
            type Error;
            type Fut<'a>: Future<Output = Result<Self::Response, Self::Error>> + Send + 'a
            where
                Self: 'a,
                T: 'a,
                Self::Request: 'a;

            fn $fn_name<'a>(
                &'a self,
                txn: &'a T,
                request: Self::Request,
            ) -> TCResult<Self::Fut<'a>> {
                let _ = (txn, request);
                Err(Self::method_not_supported($method))
            }
        }
    };
}

define_verb_handler!(HandleGet, get, Method::Get);
define_verb_handler!(HandlePut, put, Method::Put);
define_verb_handler!(HandlePost, post, Method::Post);
define_verb_handler!(HandleDelete, delete, Method::Delete);
