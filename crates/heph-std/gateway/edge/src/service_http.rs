//! Bounded HTTP/1 exchange over one private service connection.

mod exchange;
mod policy;
mod request;
mod response;

#[cfg(test)]
#[path = "service_http/tests.rs"]
mod tests;

pub use exchange::exchange;
pub use policy::ServiceHttpPolicy;
