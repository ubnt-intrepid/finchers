//! The basic facilities for testing endpoints.
//!
//! # Example
//!
//! ```
//! # #[macro_use]
//! # extern crate finchers;
//! # use finchers::test;
//! # use finchers::prelude::*;
//! # fn main() {
//! let endpoint = path!(@get / "greeting" / String)
//!     .map(|name: String| format!("Hello, {}.", name));
//!
//! // Create an instance of TestRunner from an endpoint.
//! let mut runner = test::runner(endpoint);
//!
//! let response = runner
//!     .perform("http://www.example.com/greeting/Alice")
//!     .unwrap();
//! assert_eq!(response.status().as_u16(), 200);
//! assert!(response.headers().contains_key("content-type"));
//! assert_eq!(response.body().to_utf8().unwrap(), "Hello, Alice.");
//! # }
//! ```
//!
//! Validates the result of the endpoint without converting to HTTP response.
//!
//! ```
//! # use finchers::test;
//! # use finchers::prelude::*;
//! use finchers::error::Result;
//!
//! // A user-defined type which does not implement `Output`.
//! struct Credential;
//! let endpoint = endpoint::unit().map(|| Credential);
//!
//! let mut runner = test::runner(endpoint);
//!
//! let result: Result<Credential> = runner.apply("/");
//!
//! assert!(result.is_ok());
//! ```

use std::error::Error as StdError;
use std::io;

use bytes::Buf;
use futures::{future, stream, Async, Future, Stream};
use http;
use http::header;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::{Request, Response};
use hyper::body::Payload;
use tokio::executor::{Executor, SpawnError};
use tokio::runtime::current_thread::Runtime;

use endpoint::Endpoint;
use error;
use input::ReqBody;
use output::Output;

use app::{AppFuture, AppService};
use rt::{with_set_runtime_mode, RuntimeMode};

pub use self::request::{IntoReqBody, TestRequest};
pub use self::response::TestResult;

// ====

type Task = Box<dyn Future<Item = (), Error = ()> + Send + 'static>;

struct AnnotatedRuntime<'a>(&'a mut Runtime);

impl<'a> AnnotatedRuntime<'a> {
    fn block_on<F: Future>(&mut self, mut future: F) -> Result<F::Item, F::Error> {
        self.0.block_on(future::poll_fn(move || {
            with_set_runtime_mode(RuntimeMode::CurrentThread, || future.poll())
        }))
    }
}

struct DummyExecutor(Option<Task>);

impl Executor for DummyExecutor {
    fn spawn(&mut self, task: Task) -> Result<(), SpawnError> {
        self.0 = Some(task);
        Ok(())
    }
}

fn or_insert(headers: &mut HeaderMap, name: HeaderName, value: &'static str) {
    headers
        .entry(name)
        .unwrap()
        .or_insert_with(|| HeaderValue::from_static(value));
}

/// A helper function for creating a new `TestRunner` from the specified endpoint.
pub fn runner<E>(endpoint: E) -> TestRunner<E>
where
    for<'a> E: Endpoint<'a>,
{
    TestRunner::new(endpoint).expect("failed to start the runtime")
}

/// A test runner for emulating the behavior of endpoints in the server.
///
/// It uses internally the current thread version of Tokio runtime for executing
/// asynchronous processes.
#[derive(Debug)]
pub struct TestRunner<E> {
    endpoint: E,
    rt: Runtime,
    default_headers: Option<HeaderMap>,
}

impl<E> TestRunner<E> {
    /// Create a `TestRunner` from the specified endpoint.
    pub fn new(endpoint: E) -> io::Result<TestRunner<E>>
    where
        for<'e> E: Endpoint<'e>,
    {
        Runtime::new().map(|rt| TestRunner::with_runtime(endpoint, rt))
    }

    /// Create a `TestRunner` from the specified endpoint with a Tokio runtime.
    pub fn with_runtime(endpoint: E, rt: Runtime) -> TestRunner<E>
    where
        for<'e> E: Endpoint<'e>,
    {
        TestRunner {
            endpoint,
            rt,
            default_headers: None,
        }
    }

    /// Returns a reference to the header map, whose values are set before
    /// applying the request to endpoint.
    pub fn default_headers(&mut self) -> &mut HeaderMap {
        self.default_headers.get_or_insert_with(Default::default)
    }

    /// Returns a reference to the instance of `Endpoint` owned by this runner.
    pub fn endpoint(&mut self) -> &mut E {
        &mut self.endpoint
    }

    /// Returns a reference to the Tokio runtime managed by this runner.
    pub fn runtime(&mut self) -> &mut Runtime {
        &mut self.rt
    }

    fn prepare_request(&self, request: impl TestRequest) -> http::Result<Request<ReqBody>> {
        let mut request = request.into_request()?;

        if let Some(ref default_headers) = self.default_headers {
            for (k, v) in default_headers {
                request.headers_mut().append(k, v.clone());
            }
        }

        if let Some(len) = request.body().content_length() {
            request
                .headers_mut()
                .entry(header::CONTENT_LENGTH)
                .unwrap()
                .or_insert_with(|| {
                    len.to_string()
                        .parse()
                        .expect("should be a valid header value")
                });
        }

        or_insert(request.headers_mut(), header::HOST, "localhost");
        or_insert(
            request.headers_mut(),
            header::USER_AGENT,
            concat!("finchers/", env!("CARGO_PKG_VERSION")),
        );

        Ok(request)
    }

    fn apply_inner<'a, F, R>(&'a mut self, request: impl TestRequest, f: F) -> R
    where
        E: Endpoint<'a>,
        F: FnOnce(AppFuture<'a, E>, &mut AnnotatedRuntime<'_>) -> R,
    {
        let request = self
            .prepare_request(request)
            .expect("failed to construct a request");

        let future = AppService::new(&self.endpoint).dispatch(request);

        f(future, &mut AnnotatedRuntime(&mut self.rt))
    }

    /// Applies the given request to the inner endpoint and retrieves the result of returned future.
    ///
    /// This method is available only if the output of endpoint is a tuple with a single element.
    /// If the output type is an unit or the tuple contains more than one element, use `apply_raw` instead.
    #[inline]
    pub fn apply<'a, T>(&'a mut self, request: impl TestRequest) -> error::Result<T>
    where
        E: Endpoint<'a, Output = (T,)>,
    {
        self.apply_raw(request).map(|(x,)| x)
    }

    /// Applies the given request to the inner endpoint and retrieves the result of returned future
    /// *without peeling tuples*.
    pub fn apply_raw<'a>(&'a mut self, request: impl TestRequest) -> error::Result<E::Output>
    where
        E: Endpoint<'a>,
    {
        self.apply_inner(request, |mut future, rt| {
            rt.block_on(future::poll_fn(|| future.poll_apply()))
        })
    }

    /// Applies the given request to the endpoint and convert the result
    /// into an HTTP response sent to the client.
    pub fn perform<'a>(
        &'a mut self,
        request: impl TestRequest,
    ) -> Result<Response<TestResult>, Box<dyn StdError + Send + Sync + 'static>>
    where
        E: Endpoint<'a>,
        E::Output: Output,
    {
        self.apply_inner(request, |mut future, rt| {
            let mut exec = DummyExecutor(None);
            let response = rt
                .block_on(future::poll_fn(|| future.poll_all(&mut exec)))
                .expect("DummyExecutor::spawn() never fails");
            let (parts, mut payload) = response.into_parts();

            let result = match exec.0 {
                Some(task) => TestResult::Upgraded(task),
                None => {
                    // construct ResBody
                    let content_length = payload.content_length();

                    let chunks = rt.block_on(
                        stream::poll_fn(|| match payload.poll_data() {
                            Ok(Async::Ready(data)) => Ok(Async::Ready(data.map(Buf::collect))),
                            Ok(Async::NotReady) => Ok(Async::NotReady),
                            Err(err) => Err(err),
                        }).collect(),
                    )?;

                    let trailers = rt.block_on(future::poll_fn(|| payload.poll_trailers()))?;

                    TestResult::Payload {
                        chunks,
                        trailers,
                        content_length,
                    }
                }
            };

            Ok(Response::from_parts(parts, result))
        })
    }
}

mod request {
    use http;
    use http::header;
    use http::{Request, Uri};
    use hyper::body::Body;
    use mime;
    use mime::Mime;

    use input::ReqBody;

    /// A trait representing the conversion into an HTTP request.
    ///
    /// This trait is internally used by the test runner.
    pub trait TestRequest: TestRequestImpl {}

    impl<'a> TestRequest for &'a str {}
    impl TestRequest for String {}
    impl TestRequest for Uri {}
    impl<'a> TestRequest for &'a Uri {}
    impl<T: IntoReqBody> TestRequest for Request<T> {}
    impl TestRequest for http::request::Builder {}
    impl<'a> TestRequest for &'a mut http::request::Builder {}
    impl<T, E> TestRequest for Result<T, E>
    where
        T: TestRequest,
        E: Into<http::Error>,
    {}

    pub trait TestRequestImpl {
        fn into_request(self) -> http::Result<Request<ReqBody>>;
    }

    impl<'a> TestRequestImpl for &'a str {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            (*self).parse::<Uri>()?.into_request()
        }
    }

    impl TestRequestImpl for String {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            self.parse::<Uri>()?.into_request()
        }
    }

    impl TestRequestImpl for Uri {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            (&self).into_request()
        }
    }

    impl<'a> TestRequestImpl for &'a Uri {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            let path = self.path_and_query().map(|s| s.as_str()).unwrap_or("/");
            let mut request = Request::get(path).body(ReqBody::new(Default::default()))?;

            if let Some(authority) = self.authority_part() {
                request
                    .headers_mut()
                    .entry(header::HOST)
                    .unwrap()
                    .or_insert(match authority.port() {
                        Some(port) => format!("{}:{}", authority.host(), port).parse()?,
                        None => authority.host().parse()?,
                    });
            }

            Ok(request)
        }
    }

    impl<T: IntoReqBody> TestRequestImpl for Request<T> {
        fn into_request(mut self) -> http::Result<Request<ReqBody>> {
            if let Some(mime) = self.body().content_type() {
                self.headers_mut()
                    .entry(header::CONTENT_TYPE)
                    .unwrap()
                    .or_insert(
                        mime.as_ref()
                            .parse()
                            .expect("should be a valid header value"),
                    );
            }
            Ok(self.map(|bd| bd.into_req_body()))
        }
    }

    impl TestRequestImpl for http::request::Builder {
        fn into_request(mut self) -> http::Result<Request<ReqBody>> {
            self.body(ReqBody::new(Default::default()))
        }
    }

    impl<'a> TestRequestImpl for &'a mut http::request::Builder {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            self.body(ReqBody::new(Default::default()))
        }
    }

    impl<T, E> TestRequestImpl for Result<T, E>
    where
        T: TestRequestImpl,
        E: Into<http::Error>,
    {
        fn into_request(self) -> http::Result<Request<ReqBody>> {
            self.map_err(Into::into)?.into_request()
        }
    }

    // ==== IntoReqBody ====

    /// A trait representing the conversion into a message body in HTTP requests.
    ///
    /// This trait is internally used by the test runner.
    pub trait IntoReqBody: IntoReqBodyImpl {}

    impl IntoReqBody for () {}
    impl<'a> IntoReqBody for &'a [u8] {}
    impl IntoReqBody for Vec<u8> {}
    impl<'a> IntoReqBody for &'a str {}
    impl IntoReqBody for String {}
    impl IntoReqBody for Body {}

    pub trait IntoReqBodyImpl: Sized {
        fn content_type(&self) -> Option<Mime> {
            None
        }
        fn into_req_body(self) -> ReqBody;
    }

    impl IntoReqBodyImpl for () {
        fn into_req_body(self) -> ReqBody {
            ReqBody::new(Default::default())
        }
    }

    impl<'a> IntoReqBodyImpl for &'a [u8] {
        fn into_req_body(self) -> ReqBody {
            ReqBody::new(self.to_owned().into())
        }
    }

    impl<'a> IntoReqBodyImpl for Vec<u8> {
        fn into_req_body(self) -> ReqBody {
            ReqBody::new(self.into())
        }
    }

    impl<'a> IntoReqBodyImpl for &'a str {
        fn content_type(&self) -> Option<Mime> {
            Some(mime::TEXT_PLAIN_UTF_8)
        }

        fn into_req_body(self) -> ReqBody {
            ReqBody::new(self.to_owned().into())
        }
    }

    impl IntoReqBodyImpl for String {
        fn content_type(&self) -> Option<Mime> {
            Some(mime::TEXT_PLAIN_UTF_8)
        }

        fn into_req_body(self) -> ReqBody {
            ReqBody::new(self.into())
        }
    }

    impl IntoReqBodyImpl for Body {
        fn into_req_body(self) -> ReqBody {
            ReqBody::new(self)
        }
    }
}

mod response {
    use std::borrow::Cow;
    use std::fmt;
    use std::str;

    use bytes::Bytes;
    use http::header::HeaderMap;

    use super::Task;

    /// A struct representing a response body returned from the test runner.
    #[allow(missing_docs)]
    pub enum TestResult {
        Upgraded(Task),
        Payload {
            chunks: Vec<Bytes>,
            trailers: Option<HeaderMap>,
            content_length: Option<u64>,
        },
    }

    impl fmt::Debug for TestResult {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                TestResult::Upgraded(..) => f.debug_tuple("Upgraded").finish(),
                TestResult::Payload {
                    ref chunks,
                    ref trailers,
                    ref content_length,
                } => f
                    .debug_struct("Payload")
                    .field("chunks", chunks)
                    .field("trailers", trailers)
                    .field("content_length", content_length)
                    .finish(),
            }
        }
    }

    #[allow(missing_docs)]
    impl TestResult {
        pub fn chunks(&self) -> Option<&[Bytes]> {
            match self {
                TestResult::Payload { ref chunks, .. } => Some(chunks),
                TestResult::Upgraded(..) => None,
            }
        }

        pub fn trailers(&self) -> Option<&HeaderMap> {
            match self {
                TestResult::Payload { ref trailers, .. } => trailers.as_ref(),
                TestResult::Upgraded(..) => None,
            }
        }

        pub fn content_length(&self) -> Option<u64> {
            match *self {
                TestResult::Payload { content_length, .. } => content_length,
                TestResult::Upgraded(..) => None,
            }
        }

        pub fn is_chunked(&self) -> bool {
            match self {
                TestResult::Upgraded(..) => false,
                TestResult::Payload { content_length, .. } => content_length.is_none(),
            }
        }

        pub fn is_upgraded(&self) -> bool {
            match self {
                TestResult::Upgraded(..) => true,
                TestResult::Payload { .. } => false,
            }
        }

        pub fn to_bytes(&self) -> Option<Cow<'_, [u8]>> {
            let chunks = self.chunks()?;
            Some(match chunks.len() {
                0 => Cow::Borrowed(&[]),
                1 => Cow::Borrowed(chunks[0].as_ref()),
                _ => Cow::Owned(chunks.iter().fold(Vec::new(), |mut acc, chunk| {
                    acc.extend_from_slice(&chunk);
                    acc
                })),
            })
        }

        pub fn to_utf8(&self) -> Option<Cow<'_, str>> {
            match self.to_bytes()? {
                Cow::Borrowed(bytes) => str::from_utf8(bytes).map(Cow::Borrowed).ok(),
                Cow::Owned(bytes) => String::from_utf8(bytes).map(Cow::Owned).ok(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{runner, TestRequest, TestResult};
    use endpoint;
    use endpoint::Endpoint;
    use http::header;
    use http::{Request, Response, Uri};

    #[test]
    fn test_test_request() {
        fn assert_impl<T: TestRequest>(t: T) {
            drop(t)
        }

        assert_impl("/"); // &str
        assert_impl(format!("/foo/bar")); // String
        assert_impl(Uri::from_static("http://example.com/"));
        assert_impl(&Uri::from_static("/foo/bar?count=1"));
        assert_impl(Request::get("/")); // Builder
        assert_impl(Request::post("/").header("content-type", "application/json")); // &mut Builder
        assert_impl(Request::put("/").body("text")); // Result<Response<_>, Error>
    }

    #[test]
    fn test_apply_all() {
        let mut runner = runner({ endpoint::cloned("Hello") });
        let response: Response<TestResult> = runner.perform("/").unwrap();

        assert_eq!(response.status().as_u16(), 200);
        assert!(response.headers().contains_key("content-type"));
        assert!(response.headers().contains_key("content-length"));
        assert!(response.headers().contains_key("server"));
        assert_eq!(response.body().to_utf8().unwrap_or("".into()), "Hello");
        assert!(response.body().trailers().is_none());
    }

    #[test]
    fn test_host_useragent() {
        let mut runner = runner({
            endpoint::apply_raw(|cx| {
                let host = cx.headers().get(header::HOST).cloned();
                let user_agent = cx.headers().get(header::USER_AGENT).cloned();
                Ok(Ok((host, user_agent)))
            })
        });

        assert_matches!(
            runner.apply_raw("/"),
            Ok((Some(ref host), Some(ref user_agent)))
                if host == "localhost" &&
                   user_agent.to_str().unwrap().starts_with("finchers/")
        );

        assert_matches!(
            runner.apply_raw("http://www.example.com/path/to"),
            Ok((Some(ref host), Some(ref user_agent)))
                if host == "www.example.com" &&
                   user_agent.to_str().unwrap().starts_with("finchers/")
        );

        assert_matches!(
            runner.apply_raw(
                Request::get("/path/to")
                    .header(header::USER_AGENT, "custom/0.0.0")),
            Ok((Some(ref host), Some(ref user_agent)))
                if host == "localhost" &&
                   user_agent.to_str().unwrap() == "custom/0.0.0"

        );
    }

    #[test]
    fn test_default_headers() {
        let mut runner = runner({
            endpoint::unit().wrap(endpoint::wrapper::before_apply(|cx| {
                assert!(cx.headers().contains_key(header::ORIGIN));
                Ok(())
            }))
        });
        runner
            .default_headers()
            .entry(header::ORIGIN)
            .unwrap()
            .or_insert("www.example.com".parse().unwrap());

        assert!(runner.apply_raw("/").is_ok());
    }
}
