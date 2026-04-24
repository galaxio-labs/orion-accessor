use derive_more::From;
use orion_error::{DomainReason, ErrorCode, StructError, UvsReason};
use serde_derive::Serialize;
use std::time::Duration;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, PartialEq, Error, From)]
pub enum AddrReason {
    #[error("unknown")]
    Brief(String),
    #[error("{0}")]
    Uvs(UvsReason),
    #[error("Operation timed out after {timeout:?} and {attempts} attempts")]
    OperationTimeoutExceeded { timeout: Duration, attempts: u32 },
    #[error("Total timeout {total_timeout:?} exceeded after {elapsed:?}")]
    TotalTimeoutExceeded {
        total_timeout: Duration,
        elapsed: Duration,
    },
    #[error("Retry exhausted after {attempts} attempts, last error: {last_error}")]
    RetryExhausted { attempts: u32, last_error: String },
}

impl DomainReason for AddrReason {}

impl ErrorCode for AddrReason {
    fn error_code(&self) -> i32 {
        match self {
            AddrReason::Brief(_) => 500,
            AddrReason::Uvs(r) => r.error_code(),
            AddrReason::OperationTimeoutExceeded { .. } => 408,
            AddrReason::TotalTimeoutExceeded { .. } => 408,
            AddrReason::RetryExhausted { .. } => 504,
        }
    }
}

pub type AddrResult<T> = Result<T, StructError<AddrReason>>;
pub type AddrError = StructError<AddrReason>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_addr_reason_brief() {
        let reason = AddrReason::Brief("test error".to_string());
        assert_eq!(reason.error_code(), 500);
        assert_eq!(reason.to_string(), "unknown");
    }

    #[test]
    fn test_addr_reason_operation_timeout_exceeded() {
        let timeout = Duration::from_secs(30);
        let reason = AddrReason::OperationTimeoutExceeded {
            timeout,
            attempts: 3,
        };
        assert_eq!(reason.error_code(), 408);
        let error_msg = reason.to_string();
        assert!(error_msg.contains("30s"));
        assert!(error_msg.contains("3 attempts"));
    }

    #[test]
    fn test_addr_reason_total_timeout_exceeded() {
        let total_timeout = Duration::from_secs(60);
        let elapsed = Duration::from_secs(65);
        let reason = AddrReason::TotalTimeoutExceeded {
            total_timeout,
            elapsed,
        };
        assert_eq!(reason.error_code(), 408);
        let error_msg = reason.to_string();
        assert!(error_msg.contains("60s"));
        assert!(error_msg.contains("65s"));
    }

    #[test]
    fn test_addr_reason_retry_exhausted() {
        let reason = AddrReason::RetryExhausted {
            attempts: 5,
            last_error: "connection failed".to_string(),
        };
        assert_eq!(reason.error_code(), 504);
        let error_msg = reason.to_string();
        assert!(error_msg.contains("5 attempts"));
        assert!(error_msg.contains("connection failed"));
    }
}
