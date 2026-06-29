/// The single error type surfaced across the FFI boundary.
///
/// Every fallible iroh call is collapsed into a human-readable message; the app
/// only ever logs or demotes-to-demo on failure, so a structured taxonomy buys
/// nothing here.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum IrohError {
    #[error("{msg}")]
    Generic { msg: String },
}

impl IrohError {
    /// Convenience for `.map_err(IrohError::msg)` on any displayable error.
    pub(crate) fn msg(e: impl std::fmt::Display) -> Self {
        IrohError::Generic { msg: e.to_string() }
    }
}
