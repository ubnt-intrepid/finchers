//! Definitions and reexports of incoming HTTP requests

mod body;
mod form;
mod from_body;
mod from_param;
mod parse_body;
mod request;
mod request_info;


#[doc(inline)]
pub use self::body::Body;

#[doc(inline)]
pub use self::form::{Form, FormParseError, FromForm};

#[doc(inline)]
pub use self::from_body::FromBody;

#[doc(inline)]
pub use self::from_param::FromParam;

#[doc(inline)]
pub use self::parse_body::{ParseBody, ParseBodyError};

#[doc(inline)]
pub use self::request::Request;

pub use self::request_info::RequestInfo;


use hyper;

/// reconstruct the raw incoming HTTP request, and return a pair of `Request` and `Body`
pub fn reconstruct(req: hyper::Request) -> (Request, Body) {
    let (method, uri, _version, headers, body) = req.deconstruct();
    let req = Request {
        method,
        uri,
        headers,
    };
    (req, body.into())
}
