use derive_more::From;
use orion_error::OrionError;
use orion_error::{StructError, UnifiedReason};
use serde_derive::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, PartialEq, From, OrionError)]
pub enum AddrReason {
    #[orion_error(identity = "biz.addr.brief", code = 500)]
    Brief(String),
    #[orion_error(transparent)]
    Unified(UnifiedReason),
    #[orion_error(
        identity = "biz.addr.operation_timeout",
        code = 408,
        message = "operation timed out"
    )]
    OperationTimeoutExceeded { timeout: Duration, attempts: u32 },
    #[orion_error(
        identity = "biz.addr.total_timeout",
        code = 408,
        message = "total timeout exceeded"
    )]
    TotalTimeoutExceeded {
        total_timeout: Duration,
        elapsed: Duration,
    },
    #[orion_error(
        identity = "biz.addr.retry_exhausted",
        code = 504,
        message = "retry exhausted"
    )]
    RetryExhausted { attempts: u32, last_error: String },
}

pub type AddrResult<T> = Result<T, StructError<AddrReason>>;
pub type AddrError = StructError<AddrReason>;

#[cfg(test)]
mod tests {
    use super::*;
    use orion_error::reason::ErrorCode;
    use std::time::Duration;

    #[test]
    fn test_addr_reason_brief() {
        let reason = AddrReason::Brief("test error".to_string());
        assert_eq!(reason.error_code(), 500);
        assert_eq!(reason.to_string(), "brief");
    }

    #[test]
    fn test_addr_reason_operation_timeout_exceeded() {
        let timeout = Duration::from_secs(30);
        let reason = AddrReason::OperationTimeoutExceeded {
            timeout,
            attempts: 3,
        };
        assert_eq!(reason.error_code(), 408);
        assert_eq!(reason.to_string(), "operation timed out");
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
        assert_eq!(reason.to_string(), "total timeout exceeded");
    }

    #[test]
    fn test_addr_reason_retry_exhausted() {
        let reason = AddrReason::RetryExhausted {
            attempts: 5,
            last_error: "connection failed".to_string(),
        };
        assert_eq!(reason.error_code(), 504);
        assert_eq!(reason.to_string(), "retry exhausted");
    }
}
