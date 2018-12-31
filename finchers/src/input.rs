//! Components for parsing the incoming HTTP request.

mod encoded;
mod header;

pub use self::encoded::{EncodedStr, FromEncodedStr};
pub use self::header::FromHeaderValue;

// ====

use http;
use http::header::HeaderMap;
use http::Request;
use http::Response;
use mime::Mime;

use crate::error::{bad_request, Error};

/// The contextual information with an incoming HTTP request.
#[derive(Debug)]
pub struct Input {
    request: Request<()>,
    body: Option<hyper::Body>,
    #[allow(clippy::option_option)]
    media_type: Option<Option<Mime>>,
    response_headers: Option<HeaderMap>,
}

impl Input {
    pub(crate) fn new(request: Request<hyper::Body>) -> Input {
        let (parts, body) = request.into_parts();
        Input {
            request: Request::from_parts(parts, ()),
            body: Some(body),
            media_type: None,
            response_headers: None,
        }
    }

    /// Returns a reference to the HTTP method of the request.
    pub fn method(&self) -> &http::Method {
        self.request.method()
    }

    /// Returns a reference to the URI of the request.
    pub fn uri(&self) -> &http::Uri {
        self.request.uri()
    }

    /// Returns the HTTP version of the request.
    pub fn version(&self) -> http::Version {
        self.request.version()
    }

    /// Returns a reference to the header map in the request.
    pub fn headers(&self) -> &http::HeaderMap {
        self.request.headers()
    }

    /// Returns a reference to the extension map which contains
    /// extra information about the request.
    pub fn extensions(&self) -> &http::Extensions {
        self.request.extensions()
    }

    /// Returns a mutable reference to the message body in the request.
    pub fn body(&mut self) -> &mut Option<hyper::Body> {
        &mut self.body
    }

    /// Attempts to get the entry of `Content-type` and parse its value.
    ///
    /// The result of this method is cached and it will return the reference to the cached value
    /// on subsequent calls.
    pub fn content_type(&mut self) -> Result<Option<&Mime>, Error> {
        match self.media_type {
            Some(ref m) => Ok(m.as_ref()),
            None => {
                let mime = match self.request.headers().get(http::header::CONTENT_TYPE) {
                    Some(raw) => {
                        let raw_str = raw.to_str().map_err(bad_request)?;
                        let mime = raw_str.parse().map_err(bad_request)?;
                        Some(mime)
                    }
                    None => None,
                };
                Ok(self.media_type.get_or_insert(mime).as_ref())
            }
        }
    }

    /// Returns a mutable reference to a `HeaderMap` which contains the entries of response headers.
    ///
    /// The values inserted in this header map are automatically added to the actual response.
    pub fn response_headers(&mut self) -> &mut HeaderMap {
        self.response_headers.get_or_insert_with(Default::default)
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn finalize<T>(
        self,
        output: Result<Response<T>, Error>,
    ) -> Response<Result<Option<T>, Error>> {
        let mut response = match output {
            Ok(response) => response.map(|bd| Ok(Some(bd))),
            Err(err) => err.into_response().map(Err),
        };

        if let Some(headers) = self.response_headers {
            response.headers_mut().extend(headers);
        }

        response
    }
}
