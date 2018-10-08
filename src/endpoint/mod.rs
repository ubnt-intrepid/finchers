//! Components for constructing `Endpoint`.

mod boxed;
pub mod context;
pub mod error;
pub mod syntax;
pub mod wrapper;

mod and;
mod apply;
mod by_ref;
mod cloned;
mod lazy;
mod or;
mod or_strict;
mod unit;

// re-exports
pub use self::boxed::{EndpointObj, LocalEndpointObj};
pub use self::context::{with_get_cx, ApplyContext, TaskContext};
pub(crate) use self::context::{with_set_cx, Cursor};
pub use self::error::{ApplyError, ApplyResult};
pub use self::wrapper::{EndpointWrapExt, Wrapper};

pub use self::and::And;
pub use self::or::Or;
pub use self::or_strict::OrStrict;

pub use self::apply::{apply, apply_raw, Apply, ApplyRaw};
pub use self::by_ref::{by_ref, ByRef};
pub use self::cloned::{cloned, Cloned};
pub use self::lazy::{lazy, Lazy};
pub use self::unit::{unit, Unit};

pub use self::output_endpoint::OutputEndpoint;
pub use self::send_endpoint::SendEndpoint;

// ====

use std::rc::Rc;
use std::sync::Arc;

use futures::Future;

use common::{Combine, Tuple};
use error::Error;

/// Trait representing an endpoint.
pub trait Endpoint<'a>: 'a {
    /// The inner type associated with this endpoint.
    type Output: Tuple;

    /// The type of value which will be returned from `apply`.
    type Future: Future<Item = Self::Output, Error = Error> + 'a;

    /// Perform checking the incoming HTTP request and returns
    /// an instance of the associated Future if matched.
    fn apply(&'a self, ecx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future>;

    /// Add an annotation that the associated type `Output` is fixed to `T`.
    #[inline(always)]
    fn with_output<T: Tuple>(self) -> Self
    where
        Self: Endpoint<'a, Output = T> + Sized,
    {
        self
    }

    /// Converts `self` using the provided `Wrapper`.
    fn wrap<W>(self, wrapper: W) -> W::Endpoint
    where
        Self: Sized,
        W: Wrapper<'a, Self>,
    {
        (wrapper.wrap(self)).with_output::<W::Output>()
    }
}

impl<'a, E: Endpoint<'a>> Endpoint<'a> for Box<E> {
    type Output = E::Output;
    type Future = E::Future;

    fn apply(&'a self, ecx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
        (**self).apply(ecx)
    }
}

impl<'a, E: Endpoint<'a>> Endpoint<'a> for Rc<E> {
    type Output = E::Output;
    type Future = E::Future;

    fn apply(&'a self, ecx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
        (**self).apply(ecx)
    }
}

impl<'a, E: Endpoint<'a>> Endpoint<'a> for Arc<E> {
    type Output = E::Output;
    type Future = E::Future;

    fn apply(&'a self, ecx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
        (**self).apply(ecx)
    }
}

/// Trait representing the transformation into an `Endpoint`.
pub trait IntoEndpoint<'a> {
    /// The inner type of associated `Endpoint`.
    type Output: Tuple;

    /// The type of transformed `Endpoint`.
    type Endpoint: Endpoint<'a, Output = Self::Output>;

    /// Consume itself and transform into an `Endpoint`.
    fn into_endpoint(self) -> Self::Endpoint;
}

impl<'a, E: Endpoint<'a>> IntoEndpoint<'a> for E {
    type Output = E::Output;
    type Endpoint = E;

    #[inline]
    fn into_endpoint(self) -> Self::Endpoint {
        self
    }
}

/// A set of extension methods for composing multiple endpoints.
pub trait IntoEndpointExt<'a>: IntoEndpoint<'a> + Sized {
    /// Create an endpoint which evaluates `self` and `e` and returns a pair of their tasks.
    ///
    /// The returned future from this endpoint contains both futures from
    /// `self` and `e` and resolved as a pair of values returned from theirs.
    fn and<E>(self, other: E) -> And<Self::Endpoint, E::Endpoint>
    where
        E: IntoEndpoint<'a>,
        Self::Output: Combine<E::Output>,
    {
        (And {
            e1: self.into_endpoint(),
            e2: other.into_endpoint(),
        }).with_output::<<Self::Output as Combine<E::Output>>::Out>()
    }

    /// Create an endpoint which evaluates `self` and `e` sequentially.
    ///
    /// The returned future from this endpoint contains the one returned
    /// from either `self` or `e` matched "better" to the input.
    fn or<E>(self, other: E) -> Or<Self::Endpoint, E::Endpoint>
    where
        E: IntoEndpoint<'a>,
    {
        (Or {
            e1: self.into_endpoint(),
            e2: other.into_endpoint(),
        }).with_output::<(self::or::Wrapped<Self::Output, E::Output>,)>()
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
    fn or_strict<E>(self, other: E) -> OrStrict<Self::Endpoint, E::Endpoint>
    where
        E: IntoEndpoint<'a, Output = Self::Output>,
    {
        (OrStrict {
            e1: self.into_endpoint(),
            e2: other.into_endpoint(),
        }).with_output::<Self::Output>()
    }
}

impl<'a, E: IntoEndpoint<'a>> IntoEndpointExt<'a> for E {}

mod send_endpoint {
    use futures::Future;

    use super::{ApplyContext, ApplyResult, Endpoint};
    use common::Tuple;
    use error::Error;

    /// A trait representing an endpoint with a constraint that the returned "Future"
    /// to be transferred across thread boundaries.
    pub trait SendEndpoint<'a>: 'a + Sealed {
        #[doc(hidden)]
        type Output: Tuple;
        #[doc(hidden)]
        type Future: Future<Item = Self::Output, Error = Error> + Send + 'a;
        #[doc(hidden)]
        fn apply_send(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future>;
    }

    pub trait Sealed {}

    impl<'a, E> Sealed for E
    where
        E: Endpoint<'a>,
        E::Future: Send,
    {
    }

    impl<'a, E> SendEndpoint<'a> for E
    where
        E: Endpoint<'a>,
        E::Future: Send,
    {
        type Output = E::Output;
        type Future = E::Future;

        #[inline(always)]
        fn apply_send(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
            self.apply(cx)
        }
    }
}

mod output_endpoint {
    use futures::Future;

    use common::Tuple;
    use endpoint::{ApplyContext, ApplyResult, Endpoint};
    use error::Error;
    use output::Output;

    /// A trait representing an endpoint with a constraint that the returned value
    /// can be convert into an HTTP response.
    pub trait OutputEndpoint<'a>: 'a + Sealed {
        #[doc(hidden)]
        type Output: Tuple + Output;
        #[doc(hidden)]
        type Future: Future<Item = Self::Output, Error = Error> + 'a;
        #[doc(hidden)]
        fn apply_output(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future>;
    }

    impl<'a, E> OutputEndpoint<'a> for E
    where
        E: Endpoint<'a>,
        E::Output: Output,
    {
        type Output = E::Output;
        type Future = E::Future;

        #[inline]
        fn apply_output(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
            self.apply(cx)
        }
    }

    pub trait Sealed {}

    impl<'a, E> Sealed for E
    where
        E: Endpoint<'a>,
        E::Output: Output,
    {
    }
}
