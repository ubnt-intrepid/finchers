use std::path::{Component, Components, Path};
use request::Request;


#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub struct Segments<'a>(Components<'a>);

impl<'a> From<&'a str> for Segments<'a> {
    fn from(path: &'a str) -> Self {
        let mut components = Path::new(path).components();
        components.next(); // skip the root ("/")
        Segments(components)
    }
}

impl<'a> Iterator for Segments<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|c| match c {
            Component::Normal(s) => s.to_str().unwrap(),
            _ => panic!("relatative path is not supported"),
        })
    }
}


/// A set of values, contains the incoming HTTP request and the finchers-specific context.
#[derive(Debug, Clone)]
pub struct EndpointContext<'a> {
    request: &'a Request,
    segments: Option<Segments<'a>>,
}

impl<'a> EndpointContext<'a> {
    #[allow(missing_docs)]
    pub fn new(request: &'a Request) -> Self {
        EndpointContext {
            request,
            segments: Some(Segments::from(request.path())),
        }
    }

    #[allow(missing_docs)]
    pub fn request(&self) -> &Request {
        self.request
    }

    /// Pop and return the front element of path segments.
    pub fn next_segment(&mut self) -> Option<&str> {
        self.segments.as_mut().and_then(|r| r.next())
    }

    /// Collect and return the remaining path segments, if available
    pub fn take_segments(&mut self) -> Option<Segments<'a>> {
        self.segments.take()
    }
}
