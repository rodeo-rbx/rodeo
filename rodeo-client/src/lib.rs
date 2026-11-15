//! Canonical rodeo client library.
//!
//! Mirrors `rodeo-client-ts` shape in Rust. Owns a connectrpc transport,
//! exposes `RodeoClient` → `StudioBackend` → `Studio` → `Dom` handles,
//! with `runCode` streaming + local RPC dispatch.

pub mod client;
pub mod studio;
pub mod dom;
pub mod run;
pub mod ree;
mod transport;
pub mod runtime;

pub use client::RodeoClient;
pub use studio::{
    MultiplayerTest, Studio, StudioBackend,
};
pub use dom::Dom;
pub use run::{RunCodeOpts, RunResult, RunStream};

pub use rodeo_proto as proto;
pub use rodeo_proto::BUILD_ID;
