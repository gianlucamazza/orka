//! HTTP router construction for the Orka server.
//!
//! Exposes [`router::RouterParams`] and [`router::build_router`] so that
//! both the server binary and integration tests can assemble the same
//! production router with different (e.g. in-memory) backends.
pub mod router;
