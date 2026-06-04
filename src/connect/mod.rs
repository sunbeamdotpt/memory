//! ConnectRPC adapter for sunbeam-memory.
//!
//! [`memory_proto`] contains the generated buffa types from
//! `proto/sunbeam/memory/v1/memory.proto`.
//! [`service::MemoryConnectService`] implements the generated [`MemoryService`]
//! trait by delegating to [`CoreService`].

pub mod memory_proto {
    //! Generated protobuf types for `sunbeam.memory.v1`.
    include!(concat!(env!("OUT_DIR"), "/_memory.rs"));
}

pub mod service;
