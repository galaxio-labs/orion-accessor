use crate::prelude::*;
use std::str::FromStr;

/// HTTP methods supported for upload operations
#[derive(Debug, Clone, PartialEq, Default, derive_more::Display)]
pub enum HttpMethod {
    #[display("PUT")]
    /// PUT request for binary upload
    #[default]
    Put,
    #[display("POST")]
    /// POST request for form-data upload
    Post,
    #[display("PATCH")]
    /// PATCH request for partial updates
    Patch,
}

/// 转换错误类型
#[derive(Debug, thiserror::Error)]
#[error("无效的HTTP方法: {0}")]
pub struct ParseHttpMethodError(String);

impl FromStr for HttpMethod {
    type Err = ParseHttpMethodError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "PUT" => Ok(HttpMethod::Put),
            "POST" => Ok(HttpMethod::Post),
            "PATCH" => Ok(HttpMethod::Patch),
            _ => Err(ParseHttpMethodError(s.to_string())),
        }
    }
}

impl TryFrom<&str> for HttpMethod {
    type Error = ParseHttpMethodError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        HttpMethod::from_str(value)
    }
}

impl TryFrom<String> for HttpMethod {
    type Error = ParseHttpMethodError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        HttpMethod::from_str(&value)
    }
}

/// Options for controlling upload operations, primarily focused on HTTP uploads
#[derive(Clone, Debug, Default)]
pub struct UploadOptions {
    /// HTTP method to use for upload
    http_method: HttpMethod,
    /// Whether to compress the resource before upload
    compression: bool,
    /// Additional metadata to include in upload headers
    metadata: ValueDict,
}

impl UploadOptions {
    /// Create new UploadOptions with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Create UploadOptions for specific HTTP method
    pub fn with_method(method: HttpMethod) -> Self {
        Self {
            http_method: method,
            compression: false,
            metadata: ValueDict::default(),
        }
    }

    /// Set HTTP method for upload
    pub fn method(mut self, method: HttpMethod) -> Self {
        self.http_method = method;
        self
    }

    /// Enable or disable compression
    pub fn compression(mut self, enable: bool) -> Self {
        self.compression = enable;
        self
    }

    /// Add metadata to upload
    /// 添加自定义元数据
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata
            .insert(key.into(), ValueType::String(value.into()));
        self
    }

    /// Get the HTTP method
    pub fn http_method(&self) -> &HttpMethod {
        &self.http_method
    }

    /// Check if compression is enabled
    pub fn compression_enabled(&self) -> bool {
        self.compression
    }

    /// Get metadata
    pub fn metadata_dict(&self) -> &ValueDict {
        &self.metadata
    }

    /// Create for testing purposes
    pub fn for_test() -> Self {
        Self {
            http_method: HttpMethod::Put,
            compression: false,
            metadata: ValueDict::default(),
        }
    }
}

impl From<(usize, ValueDict)> for UploadOptions {
    fn from((method, values): (usize, ValueDict)) -> Self {
        let http_method = match method {
            0 => HttpMethod::Put,
            1 => HttpMethod::Post,
            2 => HttpMethod::Patch,
            _ => HttpMethod::Put,
        };

        Self {
            http_method,
            compression: false,
            metadata: values,
        }
    }
}

impl From<crate::update::DownloadOptions> for UploadOptions {
    fn from(_: crate::update::DownloadOptions) -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_from_str_valid_cases() {
        assert_eq!(HttpMethod::from_str("PUT").unwrap(), HttpMethod::Put);
        assert_eq!(HttpMethod::from_str("put").unwrap(), HttpMethod::Put);
        assert_eq!(HttpMethod::from_str("Put").unwrap(), HttpMethod::Put);

        assert_eq!(HttpMethod::from_str("POST").unwrap(), HttpMethod::Post);
        assert_eq!(HttpMethod::from_str("post").unwrap(), HttpMethod::Post);
        assert_eq!(HttpMethod::from_str("Post").unwrap(), HttpMethod::Post);

        assert_eq!(HttpMethod::from_str("PATCH").unwrap(), HttpMethod::Patch);
        assert_eq!(HttpMethod::from_str("patch").unwrap(), HttpMethod::Patch);
        assert_eq!(HttpMethod::from_str("Patch").unwrap(), HttpMethod::Patch);
    }

    #[test]
    fn test_from_str_invalid_cases() {
        assert!(HttpMethod::from_str("GET").is_err());
        assert!(HttpMethod::from_str("DELETE").is_err());
        assert!(HttpMethod::from_str("INVALID").is_err());
        assert!(HttpMethod::from_str("").is_err());
        assert!(HttpMethod::from_str("  PUT  ").is_err()); // 包含空格
    }

    #[test]
    fn test_try_from_str() {
        assert_eq!(HttpMethod::try_from("PUT").unwrap(), HttpMethod::Put);
        assert_eq!(HttpMethod::try_from("Post").unwrap(), HttpMethod::Post);
        assert_eq!(HttpMethod::try_from("patch").unwrap(), HttpMethod::Patch);
    }

    #[test]
    fn test_try_from_string() {
        assert_eq!(
            HttpMethod::try_from(String::from("PUT")).unwrap(),
            HttpMethod::Put
        );
        assert_eq!(
            HttpMethod::try_from(String::from("POST")).unwrap(),
            HttpMethod::Post
        );
        assert_eq!(
            HttpMethod::try_from(String::from("PATCH")).unwrap(),
            HttpMethod::Patch
        );

        assert!(HttpMethod::try_from(String::from("DELETE")).is_err());
        assert!(HttpMethod::try_from(String::from("")).is_err());
    }

    #[test]
    fn test_parse_http_method_error() {
        let err = HttpMethod::from_str("INVALID").unwrap_err();
        assert_eq!(err.to_string(), "无效的HTTP方法: INVALID");

        let err = HttpMethod::from_str("").unwrap_err();
        assert_eq!(err.to_string(), "无效的HTTP方法: ");
    }

    #[test]
    fn test_http_method_display() {
        assert_eq!(HttpMethod::Put.to_string(), "PUT");
        assert_eq!(HttpMethod::Post.to_string(), "POST");
        assert_eq!(HttpMethod::Patch.to_string(), "PATCH");
    }

    #[test]
    fn test_http_method_default() {
        let method = HttpMethod::default();
        assert_eq!(method, HttpMethod::Put);
    }

    #[test]
    fn test_http_method_clone() {
        let original = HttpMethod::Post;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_http_method_partial_eq() {
        assert_eq!(HttpMethod::Put, HttpMethod::Put);
        assert_ne!(HttpMethod::Put, HttpMethod::Post);
        assert_ne!(HttpMethod::Post, HttpMethod::Patch);
        assert_ne!(HttpMethod::Patch, HttpMethod::Put);
    }

    #[test]
    fn test_upload_options_new() {
        let options = UploadOptions::new();
        assert_eq!(options.http_method(), &HttpMethod::Put);
        assert!(!options.compression_enabled());
        assert!(options.metadata_dict().is_empty());
    }

    #[test]
    fn test_upload_options_default() {
        let options = UploadOptions::default();
        assert_eq!(options.http_method(), &HttpMethod::Put);
        assert!(!options.compression_enabled());
        assert!(options.metadata_dict().is_empty());
    }

    #[test]
    fn test_upload_options_with_method() {
        let options = UploadOptions::with_method(HttpMethod::Post);
        assert_eq!(options.http_method(), &HttpMethod::Post);
        assert!(!options.compression_enabled());
        assert!(options.metadata_dict().is_empty());
    }

    #[test]
    fn test_upload_options_method_setter() {
        let options = UploadOptions::new().method(HttpMethod::Patch);
        assert_eq!(options.http_method(), &HttpMethod::Patch);
    }

    #[test]
    fn test_upload_options_compression() {
        let options = UploadOptions::new().compression(true);
        assert!(options.compression_enabled());

        let options = options.compression(false);
        assert!(!options.compression_enabled());
    }

    #[test]
    fn test_upload_options_metadata() {
        let options = UploadOptions::new()
            .metadata("key1", "value1")
            .metadata("key2", "value2");

        let metadata = options.metadata_dict();
        assert_eq!(metadata.len(), 2);
        assert_eq!(
            metadata.get("KEY1"),
            Some(&ValueType::String("value1".to_string()))
        );
        assert_eq!(
            metadata.get("KEY2"),
            Some(&ValueType::String("value2".to_string()))
        );
    }

    #[test]
    fn test_upload_options_for_test() {
        let options = UploadOptions::for_test();
        assert_eq!(options.http_method(), &HttpMethod::Put);
        assert!(!options.compression_enabled());
        assert!(options.metadata_dict().is_empty());
    }

    #[test]
    fn test_upload_options_getters() {
        let options = UploadOptions::new()
            .method(HttpMethod::Post)
            .compression(true)
            .metadata("test", "value");

        assert_eq!(options.http_method(), &HttpMethod::Post);
        assert!(options.compression_enabled());
        assert_eq!(options.metadata_dict().len(), 1);
    }

    #[test]
    fn test_upload_options_clone() {
        let original = UploadOptions::new()
            .method(HttpMethod::Patch)
            .compression(true)
            .metadata("clone", "test");

        let cloned = original.clone();
        assert_eq!(original.http_method(), cloned.http_method());
        assert_eq!(original.compression_enabled(), cloned.compression_enabled());
        assert_eq!(original.metadata_dict(), cloned.metadata_dict());
    }

    #[test]
    fn test_upload_options_debug() {
        let options = UploadOptions::new()
            .method(HttpMethod::Post)
            .compression(true)
            .metadata("debug", "test");

        let debug_str = format!("{options:?}");
        assert!(debug_str.contains("UploadOptions"));
        assert!(debug_str.contains("Post"));
        assert!(debug_str.contains("true"));
    }

    #[test]
    fn test_from_usize_value_dict() {
        let mut metadata = ValueDict::new();
        metadata.insert("test".to_string(), ValueType::String("value".to_string()));

        let options: UploadOptions = (0, metadata.clone()).into();
        assert_eq!(options.http_method(), &HttpMethod::Put);
        assert_eq!(options.metadata_dict(), &metadata);

        let options: UploadOptions = (1, metadata.clone()).into();
        assert_eq!(options.http_method(), &HttpMethod::Post);

        let options: UploadOptions = (2, metadata.clone()).into();
        assert_eq!(options.http_method(), &HttpMethod::Patch);

        let options: UploadOptions = (999, metadata.clone()).into();
        assert_eq!(options.http_method(), &HttpMethod::Put); // 默认值
    }

    #[test]
    fn test_from_download_options() {
        // 注意：这里我们无法创建 DownloadOptions 实例，因为它在另一个模块中
        // 但我们可以测试 From trait 的存在性
        let _ = UploadOptions::from(crate::update::DownloadOptions::default());
    }

    #[test]
    fn test_upload_options_builder_pattern() {
        let options = UploadOptions::new()
            .method(HttpMethod::Post)
            .compression(true)
            .metadata("author", "test")
            .metadata("version", "1.0");

        assert_eq!(options.http_method(), &HttpMethod::Post);
        assert!(options.compression_enabled());
        assert_eq!(options.metadata_dict().len(), 2);
    }

    #[test]
    fn test_upload_options_chaining() {
        let base = UploadOptions::new();
        let options1 = base.clone().method(HttpMethod::Patch);
        let options2 = base.clone().compression(true);
        let options3 = base.clone().metadata("key", "value");

        assert_eq!(options1.http_method(), &HttpMethod::Patch);
        assert!(!options1.compression_enabled());

        assert_eq!(options2.http_method(), &HttpMethod::Put);
        assert!(options2.compression_enabled());

        assert_eq!(options3.http_method(), &HttpMethod::Put);
        assert!(!options3.compression_enabled());
        assert_eq!(options3.metadata_dict().len(), 1);
    }
}
