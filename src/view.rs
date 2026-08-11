use std::future::Future;

use tc_error::TCResult;

use crate::Transaction;

/// Acquire a transaction-consistent view of a native value.
///
/// Native routing and graph execution exchange the value itself. A terminal
/// boundary acquires a view, then independently chooses how to represent it.
pub trait IntoView: Sized {
    type Txn: Transaction + Clone;
    type View: Sized + Send;

    fn into_view(self, txn: Self::Txn) -> impl Future<Output = TCResult<Self::View>> + Send;
}
