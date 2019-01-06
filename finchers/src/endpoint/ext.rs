#![allow(missing_docs)]

mod and;
mod and_then;
mod err_into;
mod map;
mod map_err;
mod or;
mod or_else;
mod or_strict;

pub use self::{
    and::And, //
    and_then::AndThen,
    err_into::ErrInto,
    map::Map,
    map_err::MapErr,
    or::Or,
    or_else::OrElse,
    or_strict::OrStrict,
};

use {
    super::IsEndpoint, //
    crate::error::Error,
};

/// A set of extension methods for combining the multiple endpoints.
pub trait EndpointExt: IsEndpoint + Sized {
    /// Create an endpoint which evaluates `self` and `e` and returns a pair of their tasks.
    ///
    /// The returned future from this endpoint contains both futures from
    /// `self` and `e` and resolved as a pair of values returned from theirs.
    fn and<E>(self, other: E) -> And<Self, E> {
        And {
            e1: self,
            e2: other,
        }
    }

    /// Create an endpoint which evaluates `self` and `e` sequentially.
    ///
    /// The returned future from this endpoint contains the one returned
    /// from either `self` or `e` matched "better" to the input.
    fn or<E>(self, other: E) -> Or<Self, E> {
        Or {
            e1: self,
            e2: other,
        }
    }

    /// Create an endpoint which evaluates `self` and `e` sequentially.
    ///
    /// The differences of behaviour to `Or` are as follows:
    ///
    /// * The associated type `E::Output` must be equal to `Self::Output`.
    ///   It means that the generated endpoint has the same output type
    ///   as the original endpoints and the return value will be used later.
    /// * If `self` is matched to the request, `other.apply(cx)`
    ///   is not called and the future returned from `self.apply(cx)` is
    ///   immediately returned.
    fn or_strict<E>(self, other: E) -> OrStrict<Self, E> {
        OrStrict {
            e1: self,
            e2: other,
        }
    }

    #[allow(missing_docs)]
    fn map<F>(self, f: F) -> Map<Self, F> {
        Map { endpoint: self, f }
    }

    #[allow(missing_docs)]
    fn and_then<F>(self, f: F) -> AndThen<Self, F> {
        AndThen { endpoint: self, f }
    }

    #[allow(missing_docs)]
    fn or_else<F>(self, f: F) -> OrElse<Self, F> {
        OrElse { endpoint: self, f }
    }

    #[allow(missing_docs)]
    fn err_into<E>(self) -> ErrInto<Self, E>
    where
        E: Into<Error>,
    {
        ErrInto {
            endpoint: self,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E: IsEndpoint> EndpointExt for E {}
