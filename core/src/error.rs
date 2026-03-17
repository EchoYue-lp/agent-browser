//! Error types for browser operations.

use thiserror::Error;

/// Browser operation errors.
#[derive(Debug, Error)]
pub enum Error {
    /// Browser launch failed.
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    /// Browser not launched.
    #[error("Browser not launched")]
    NotLaunched,

    /// Browser unresponsive.
    #[error("Browser unresponsive")]
    Unresponsive,

    /// No active page.
    #[error("No active page, please use browser_navigate first")]
    NoActivePage,

    /// Element not found.
    #[error("Element not found: ref_id={0}")]
    ElementNotFound(String),

    /// Page changed.
    #[error("Page changed: expected {expected}, current {current}")]
    PageChanged { expected: String, current: String },

    /// Operation timeout.
    #[error("Operation timeout: {0}")]
    Timeout(String),

    /// CDP error.
    #[error("CDP error: {0}")]
    Cdp(String),

    /// JavaScript execution error.
    #[error("JavaScript error: {0}")]
    JavaScript(String),

    /// Invalid parameter.
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialize(#[from] serde_json::Error),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other error.
    #[error("{0}")]
    Other(String),
}

/// Result type alias.
pub type Result<T> = std::result::Result<T, Error>;

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Other(e.to_string())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::LaunchFailed("Chrome not found".to_string());
        assert_eq!(err.to_string(), "Browser launch failed: Chrome not found");

        let err = Error::NotLaunched;
        assert_eq!(err.to_string(), "Browser not launched");

        let err = Error::NoActivePage;
        assert_eq!(
            err.to_string(),
            "No active page, please use browser_navigate first"
        );
    }

    #[test]
    fn test_error_element_not_found() {
        let err = Error::ElementNotFound("ax42".to_string());
        assert_eq!(err.to_string(), "Element not found: ref_id=ax42");
    }

    #[test]
    fn test_error_page_changed() {
        let err = Error::PageChanged {
            expected: "https://example.com/page1".to_string(),
            current: "https://example.com/page2".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Page changed"));
        assert!(msg.contains("page1"));
        assert!(msg.contains("page2"));
    }

    #[test]
    fn test_error_timeout() {
        let err = Error::Timeout("Operation timed out after 30s".to_string());
        assert_eq!(
            err.to_string(),
            "Operation timeout: Operation timed out after 30s"
        );
    }

    #[test]
    fn test_error_cdp() {
        let err = Error::Cdp("Connection closed".to_string());
        assert_eq!(err.to_string(), "CDP error: Connection closed");
    }

    #[test]
    fn test_error_javascript() {
        let err = Error::JavaScript("undefined is not a function".to_string());
        assert_eq!(
            err.to_string(),
            "JavaScript error: undefined is not a function"
        );
    }

    #[test]
    fn test_error_invalid_parameter() {
        let err = Error::InvalidParameter("url cannot be empty".to_string());
        assert_eq!(err.to_string(), "Invalid parameter: url cannot be empty");
    }

    #[test]
    fn test_error_from_serde_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
        let err: Error = json_err.unwrap_err().into();
        assert!(matches!(err, Error::Serialize(_)));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn test_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let err: Error = anyhow_err.into();
        assert!(matches!(err, Error::Other(_)));
    }

    #[test]
    fn test_result_type() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }

        fn returns_err() -> Result<i32> {
            Err(Error::Timeout("test".to_string()))
        }

        assert!(returns_ok().is_ok());
        assert!(returns_err().is_err());
    }
}
