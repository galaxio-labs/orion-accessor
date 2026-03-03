use std::time::Duration;

use crate::addr::{AddrReason, AddrResult, access_ctrl::UnitCtrl};
use orion_error::{ErrorWith, ToStructError, UvsFrom};

use reqwest::{ClientBuilder, Proxy};

pub fn create_http_client_by_ctrl(ctrl: Option<UnitCtrl>) -> AddrResult<reqwest::Client> {
    // 使用 UnitCtrl 中的超时配置创建客户端
    let mut builder = if let Some(timeout) = ctrl.as_ref().and_then(|x| x.timeout().clone()) {
        ClientBuilder::new()
            .connect_timeout(timeout.connect_duration())
            .read_timeout(timeout.read_duration())
            .timeout(timeout.total_duration())
            .tcp_keepalive(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
    } else {
        ClientBuilder::new()
    };
    if let Some(proxy) = ctrl.as_ref().and_then(|x| x.proxy().clone()) {
        if let Ok(proxy) = Proxy::all(proxy.url().as_str()) {
            builder = builder.proxy(proxy);
        } else {
            tracing::warn!("无效的代理设置: {}", proxy.url());
        }
    }
    builder.build().map_err(|e| {
        tracing::error!("创建HTTP客户端失败: {}", e);
        AddrReason::from_res()
            .to_err()
            .want(format!("create HTTP client failed: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::access_ctrl::UnitCtrl;
    use crate::addr::proxy::ProxyConfig;
    use crate::timeout::TimeoutConfig;

    #[test]
    fn test_create_http_client_by_ctrl_none() {
        // 测试当传入None时，应创建默认客户端
        let client = create_http_client_by_ctrl(None);
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_http_client_by_ctrl_with_timeout() {
        // 测试超时配置是否正确应用
        let timeout_config = TimeoutConfig::http_simple();
        let unit_ctrl = UnitCtrl::new(None, Some(timeout_config), None);
        let client = create_http_client_by_ctrl(Some(unit_ctrl));
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_http_client_by_ctrl_with_proxy() {
        // 测试代理配置是否正确应用
        let proxy_config = ProxyConfig::new("http://proxy.example.com:8080");
        let unit_ctrl = UnitCtrl::new(None, None, Some(proxy_config));
        let client = create_http_client_by_ctrl(Some(unit_ctrl));
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_http_client_by_ctrl_with_invalid_proxy() {
        // 测试无效代理配置的情况
        let proxy_config = ProxyConfig::new(""); // 空URL应该是无效的
        let unit_ctrl = UnitCtrl::new(None, None, Some(proxy_config));
        let client = create_http_client_by_ctrl(Some(unit_ctrl));
        // 即使代理无效也应该创建成功（会记录警告但不会panic）
        assert!(client.is_ok());
    }

    #[test]
    fn test_create_http_client_by_ctrl_with_all_configs() {
        // 测试同时包含超时和代理配置的情况
        let timeout_config = TimeoutConfig::http_large_file();
        let proxy_config = ProxyConfig::new("http://proxy.example.com:8080");
        let unit_ctrl = UnitCtrl::new(None, Some(timeout_config), Some(proxy_config));
        let client = create_http_client_by_ctrl(Some(unit_ctrl));
        assert!(client.is_ok());
    }
}
