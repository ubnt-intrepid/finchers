use finchers_core::endpoint::{Context, EndpointBase};
use finchers_core::input::{with_set_cx, Input};
use finchers_core::task::Task;
use finchers_core::Poll;

/// Create an asynchronous computation for handling an HTTP request.
pub fn apply_request<E>(endpoint: &E, input: &Input) -> ApplyRequest<E::Task>
where
    E: EndpointBase + ?Sized,
{
    let in_flight = endpoint.apply(&mut Context::new(input));
    ApplyRequest { in_flight }
}

/// An asynchronous computation created by the endpoint.
///
/// Typically, this value is wrapped by a type which contains an instance of `Input`.
#[derive(Debug)]
pub struct ApplyRequest<T> {
    in_flight: Option<T>,
}

impl<T: Task> ApplyRequest<T> {
    /// Poll the inner `Task` and return its output if available.
    pub fn poll_ready(&mut self, input: &mut Input) -> Poll<Option<T::Output>> {
        match self.in_flight {
            Some(ref mut f) => with_set_cx(input, || f.poll_task().map(Some)),
            None => Poll::Ready(None),
        }
    }
}
