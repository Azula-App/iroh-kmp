//! `iroh-kmp` — a minimal UniFFI wrapper around [`iroh`] for Kotlin Multiplatform.
//!
//! The surface is deliberately small: it exposes exactly what the azula app's
//! `IrohTransport` needs (bind an endpoint, hand out a ticket, dial a peer,
//! accept incoming connections, and exchange bytes over a bidirectional stream).
//! It is modeled on `n0-computer/iroh-ffi` but uses a JNI-backed Gobley binding,
//! whose async calls — unlike the published JNA artifact — complete on Android.

mod endpoint;
mod error;
mod stream;

#[cfg(target_os = "android")]
mod android_init;

pub use endpoint::{IncomingConn, IrohEndpoint};
pub use error::IrohError;
pub use stream::IrohStream;

uniffi::setup_scaffolding!();
