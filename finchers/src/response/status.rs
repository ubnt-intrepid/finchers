use http::StatusCode;

/// A trait for constructing an HTTP response from the value.
pub trait HttpStatus {
    /// Returns a HTTP status code associated with this type
    fn status_code(&self) -> StatusCode;
}

macro_rules! impl_http_response_for_types {
    ($($t:ty;)*) => {$(
        impl HttpStatus for $t {
            fn status_code(&self) -> StatusCode {
                StatusCode::OK
            }
        }
    )*};
}

impl_http_response_for_types! {
    (); bool; char; f32; f64;
    i8; i16; i32; i64; isize;
    u8; u16; u32; u64; usize;
    &'static str;
    String;
    ::std::borrow::Cow<'static, str>;
}