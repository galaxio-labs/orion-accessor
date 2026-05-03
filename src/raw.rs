use orion_error::interop::{RawSource, RawStdError, raw_source};
use std::error::Error as StdError;
use std::fmt;

/// Local wrapper that implements `RawStdError` for any `StdError`.
pub struct RawErr<E: StdError + Send + Sync + 'static>(pub E);

impl<E: StdError + Send + Sync + 'static> fmt::Display for RawErr<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<E: StdError + Send + Sync + 'static> fmt::Debug for RawErr<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl<E: StdError + Send + Sync + 'static> StdError for RawErr<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

impl<E: StdError + Send + Sync + 'static> RawStdError for RawErr<E> {}

/// Wrap any `StdError` into `RawSource<RawErr<E>>` for use with `.source_err()`.
pub fn raw_err<E: StdError + Send + Sync + 'static>(e: E) -> RawSource<RawErr<E>> {
    raw_source(RawErr(e))
}
