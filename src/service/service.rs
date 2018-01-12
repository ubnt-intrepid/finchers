#![allow(missing_docs)]

use std::io;
use std::mem;
use std::sync::Arc;

use futures::{Async, Future, Poll};
use hyper;
use hyper::server::{NewService, Service};

use http;
use endpoint::{Endpoint, EndpointContext};
use task::{Task, TaskContext};
use process::Process;
use responder::{IntoResponder, Responder};

/// The inner representation of `EndpointService`.
#[derive(Debug)]
struct EndpointServiceContext<E, P> {
    endpoint: E,
    process: Arc<P>,
}

/// An HTTP service which wraps a `Endpoint`.
#[derive(Debug)]
pub struct EndpointService<E, P>
where
    E: Endpoint,
    P: Process<In = E::Item, InErr = E::Error>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    inner: Arc<EndpointServiceContext<E, P>>,
}

impl<E, P> Service for EndpointService<E, P>
where
    E: Endpoint,
    P: Process<In = E::Item, InErr = E::Error>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    type Request = hyper::Request;
    type Response = hyper::Response;
    type Error = hyper::Error;
    type Future = EndpointServiceFuture<<E::Task as Task>::Future, P>;

    fn call(&self, req: hyper::Request) -> Self::Future {
        let inner = self.inner.endpoint.apply_request(req);
        EndpointServiceFuture {
            inner: Inner::PollingTask(inner, self.inner.process.clone()),
        }
    }
}

/// A future returned from `EndpointService::call()`
#[allow(missing_debug_implementations)]
pub struct EndpointServiceFuture<F, P: Process> {
    inner: Inner<F, P>,
}

impl<F, E, P> Future for EndpointServiceFuture<F, P>
where
    F: Future<Error = Result<E, hyper::Error>>,
    P: Process<In = F::Item, InErr = E>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    type Item = hyper::Response;
    type Error = hyper::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(item)) => Ok(Async::Ready(item.into_responder().respond())),
            Err(Ok(err)) => Ok(Async::Ready(err.into_responder().respond())),
            Err(Err(err)) => Err(err),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Inner<F, P: Process> {
    PollingTask(Option<F>, Arc<P>),
    PollingResult(P::Future),
    Done,
}

impl<F, P, E> Future for Inner<F, P>
where
    F: Future<Error = Result<E, hyper::Error>>,
    P: Process<In = F::Item, InErr = E>,
{
    type Item = <P::Future as Future>::Item;
    type Error = Result<<P::Future as Future>::Error, hyper::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        use self::Inner::*;
        loop {
            match mem::replace(self, Done) {
                PollingTask(t, p) => {
                    let input = match t {
                        Some(mut t) => {
                            let polled = t.poll();
                            match polled {
                                Ok(Async::Ready(item)) => Some(Ok(item)),
                                Ok(Async::NotReady) => {
                                    *self = PollingTask(Some(t), p);
                                    return Ok(Async::NotReady);
                                }
                                Err(Ok(err)) => Some(Err(err)),
                                Err(Err(err)) => return Err(Err(err)),
                            }
                        }
                        None => None,
                    };
                    *self = PollingResult(p.call(input));
                    continue;
                }
                PollingResult(mut p) => {
                    let polled = p.poll();
                    match polled {
                        Ok(Async::Ready(item)) => break Ok(Async::Ready(item)),
                        Ok(Async::NotReady) => {
                            *self = PollingResult(p);
                            return Ok(Async::NotReady);
                        }
                        Err(err) => break Err(Ok(err)),
                    }
                }
                Done => panic!(),
            }
        }
    }
}

#[derive(Debug)]
pub struct EndpointServiceFactory<E, P>
where
    E: Endpoint,
    P: Process<In = E::Item, InErr = E::Error>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    inner: Arc<EndpointServiceContext<E, P>>,
}

impl<E, P> EndpointServiceFactory<E, P>
where
    E: Endpoint,
    P: Process<In = E::Item, InErr = E::Error>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    pub fn new(endpoint: E, process: P) -> Self {
        EndpointServiceFactory {
            inner: Arc::new(EndpointServiceContext {
                endpoint,
                process: Arc::new(process),
            }),
        }
    }
}

impl<E, P> NewService for EndpointServiceFactory<E, P>
where
    E: Endpoint,
    P: Process<In = E::Item, InErr = E::Error>,
    P::Out: IntoResponder,
    P::OutErr: IntoResponder,
{
    type Request = hyper::Request;
    type Response = hyper::Response;
    type Error = hyper::Error;
    type Instance = EndpointService<E, P>;

    fn new_service(&self) -> io::Result<Self::Instance> {
        Ok(EndpointService {
            inner: self.inner.clone(),
        })
    }
}

pub trait EndpointExt: Endpoint + sealed::Sealed {
    fn apply_request(&self, request: hyper::Request) -> Option<<Self::Task as Task>::Future> {
        let (mut request, body) = http::request::reconstruct(request);

        let task = {
            let mut ctx = EndpointContext::new(&request);
            try_opt!(self.apply(&mut ctx))
        };

        let mut ctx = TaskContext {
            request: &mut request,
            body: Some(body),
        };

        Some(task.launch(&mut ctx))
    }
}

impl<E: Endpoint> EndpointExt for E {}

mod sealed {
    use endpoint::Endpoint;

    pub trait Sealed {}

    impl<E: Endpoint> Sealed for E {}
}
