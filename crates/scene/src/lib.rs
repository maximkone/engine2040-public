//! scene: persistent identity and content versioning for authored objects.
//!
//! Identity (`ObjectId`) is stable across reimports; the content digest
//! ([`content_digest`]) versions the bytes. Never use one for the other.

pub mod codec;
pub mod component;
pub mod ids;
/// Run pins for pre-start gates (see [`pin::ScenePin`]).
pub mod pin;
pub mod prefab;
pub mod provenance;
pub mod schema;
pub mod transform;

#[cfg(test)]
mod wiring_tests;
