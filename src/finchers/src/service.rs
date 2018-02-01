//! Components of lower-level HTTP services

use std::mem;
use std::string::ToString;
use futures::{Future, IntoFuture, Poll};
use futures::Async::*;
use http::header;
use hyper::{self, Request, Response};
use hyper::server::Service;

use core::{HttpResponse, Outcome};
use endpoint::{Endpoint, EndpointResult};
use handler::{DefaultHandler, Handler};
use responder::{DefaultResponder, Responder};

/// An HTTP service which wraps a `Endpoint`, `Handler` and `Responder`.
#[derive(Debug, Copy, Clone)]
pub struct FinchersService<E, H, R> {
    endpoint: E,
    handler: H,
    responder: R,
}

impl<E, H, R> FinchersService<E, H, R> {
    /// Create an instance of `FinchersService` from components
    pub fn new(endpoint: E, handler: H, responder: R) -> Self {
        Self {
            endpoint,
            handler,
            responder,
        }
    }
}

impl<E, H, R> Service for FinchersService<E, H, R>
where
    E: Endpoint,
    H: Handler<E::Item> + Clone,
    R: Responder<H::Item> + Clone,
{
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FinchersServiceFuture<E, H, R>;

    fn call(&self, request: Self::Request) -> Self::Future {
        let state = match self.endpoint.apply_request(request) {
            Some(input) => State::PollingInput {
                input,
                handler: self.handler.clone(),
            },
            None => State::NoRoute,
        };
        FinchersServiceFuture {
            state,
            responder: self.responder.clone(),
        }
    }
}

/// A future returned from `EndpointService::call()`
#[allow(missing_debug_implementations)]
pub struct FinchersServiceFuture<E, H, R>
where
    E: Endpoint,
    H: Handler<E::Item>,
    R: Responder<H::Item>,
{
    state: State<E, H>,
    responder: R,
}

#[allow(missing_debug_implementations)]
enum State<E, H>
where
    E: Endpoint,
    H: Handler<E::Item>,
{
    NoRoute,
    PollingInput {
        input: <E::Result as EndpointResult>::Future,
        handler: H,
    },
    PollingOutput {
        output: <H::Result as IntoFuture>::Future,
    },
    Done,
}

impl<E, H, R> FinchersServiceFuture<E, H, R>
where
    E: Endpoint,
    H: Handler<E::Item>,
    R: Responder<H::Item>,
{
    fn poll_state(&mut self) -> Poll<Outcome<H::Item>, hyper::Error> {
        use self::State::*;
        loop {
            match mem::replace(&mut self.state, Done) {
                NoRoute => break Ok(Ready(Outcome::NoRoute)),
                PollingInput { mut input, handler } => match input.poll() {
                    Ok(Ready(input)) => {
                        self.state = PollingOutput {
                            output: IntoFuture::into_future(handler.call(input)),
                        };
                        continue;
                    }
                    Ok(NotReady) => {
                        self.state = PollingInput { input, handler };
                        break Ok(NotReady);
                    }
                    Err(err) => break Ok(Ready(Outcome::Err(err))),
                },
                PollingOutput { mut output } => match output.poll() {
                    Ok(Ready(Some(item))) => break Ok(Ready(Outcome::Ok(item))),
                    Ok(Ready(None)) => break Ok(Ready(Outcome::NoRoute)),
                    Ok(NotReady) => {
                        self.state = PollingOutput { output };
                        break Ok(NotReady);
                    }
                    Err(err) => break Ok(Ready(Outcome::Err(err.into()))),
                },
                Done => panic!(),
            }
        }
    }
}

impl<E, H, R> Future for FinchersServiceFuture<E, H, R>
where
    E: Endpoint,
    H: Handler<E::Item>,
    R: Responder<H::Item>,
{
    type Item = Response;
    type Error = hyper::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let output = try_ready!(self.poll_state());
        let mut response = self.responder.respond(output);
        if !response.headers().contains_key(header::SERVER) {
            response
                .headers_mut()
                .insert(header::SERVER, "Finchers".parse().unwrap());
        }
        let response = response.map(Into::into).into();
        Ok(Ready(response))
    }
}

#[allow(missing_docs)]
pub trait EndpointServiceExt: Endpoint + sealed::Sealed
where
    Self::Item: ToString + HttpResponse,
{
    fn into_service(self) -> FinchersService<Self, DefaultHandler, DefaultResponder>
    where
        Self: Sized;

    fn with_handler<H>(self, handler: H) -> FinchersService<Self, H, DefaultResponder>
    where
        H: Handler<Self::Item> + Clone,
        Self: Sized;
}

impl<E: Endpoint> EndpointServiceExt for E
where
    E::Item: ToString + HttpResponse,
{
    fn into_service(self) -> FinchersService<Self, DefaultHandler, DefaultResponder> {
        FinchersService::new(self, DefaultHandler::default(), Default::default())
    }

    fn with_handler<H>(self, handler: H) -> FinchersService<Self, H, DefaultResponder>
    where
        H: Handler<Self::Item> + Clone,
    {
        FinchersService::new(self, handler, Default::default())
    }
}

mod sealed {
    use endpoint::Endpoint;
    pub trait Sealed {}
    impl<E: Endpoint> Sealed for E {}
}