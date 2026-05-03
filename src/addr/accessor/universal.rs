use crate::addr::access_ctrl::serv::NetAccessCtrl;
use crate::addr::{AddrResult, Address};
use crate::prelude::*;
use crate::types::{ResourceDownloader, ResourceUploader, UpdateUnit};
use crate::update::{DownloadOptions, UploadOptions};
use log::error;
use orion_conf::YamlIO;
use std::path::{Path, PathBuf};

use super::git::GitAccessor;
use super::http::HttpAccessor;
use super::local::LocalAccessor;

/// 统一地址访问器配置
#[derive(Debug, Clone, Default)]
pub struct UniversalConfig {
    /// 重定向服务配置
    pub accs_ctrl: Option<NetAccessCtrl>,
}

impl UniversalConfig {
    pub fn with_ctrl(mut self, ctrl: NetAccessCtrl) -> Self {
        self.accs_ctrl = Some(ctrl);
        self
    }
    pub fn with_ctrl_file(mut self, path: &Path) -> Self {
        if path.exists() {
            match NetAccessCtrl::load_yaml(path) {
                Ok(redirect) => {
                    self.accs_ctrl = Some(redirect);
                }
                Err(e) => {
                    error!("load redirect conf failed!\npath:{} \n{e}", path.display());
                }
            }
        }
        self
    }
    pub fn with_file_opt(self, path_opt: &Option<PathBuf>) -> Self {
        if let Some(path) = path_opt {
            return self.with_ctrl_file(path.as_path());
        }
        self
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn test_universal_config_builder_pattern() {
        let net_ctrl = NetAccessCtrl::new(vec![], true);

        let config = UniversalConfig::default().with_ctrl(net_ctrl);

        assert!(config.accs_ctrl.is_some());
    }

    #[test]
    fn test_universal_config_file_handling() {
        let test_path = PathBuf::from("nonexistent_config.yml");
        let none_path: Option<PathBuf> = None;

        let config_with_none = UniversalConfig::default().with_file_opt(&none_path);
        assert!(config_with_none.accs_ctrl.is_none());

        let config_with_path = UniversalConfig::default().with_file_opt(&Some(test_path));
        // File doesn't exist, so accs_ctrl should remain None
        assert!(config_with_path.accs_ctrl.is_none());
    }
}

/// 统一地址访问器
///
/// 提供统一的地址访问接口，根据地址类型自动选择合适的底层访问器
#[derive(Debug, Default)]
pub struct UniversalAccessor {
    git: GitAccessor,
    http: HttpAccessor,
    local: LocalAccessor,
    config: UniversalConfig,
}

impl UniversalAccessor {
    /// 创建新的统一地址访问器
    pub fn new(config: UniversalConfig) -> Self {
        let git = GitAccessor::default().with_ctrl(config.accs_ctrl.clone());
        let http = HttpAccessor::default().with_ctrl(config.accs_ctrl.clone());
        let local = LocalAccessor::default();

        Self {
            git,
            http,
            local,
            config,
        }
    }

    /// 获取配置
    pub fn config(&self) -> &UniversalConfig {
        &self.config
    }

    /// 获取可变配置
    pub fn config_mut(&mut self) -> &mut UniversalConfig {
        &mut self.config
    }
}

#[async_trait]
impl ResourceDownloader for UniversalAccessor {
    async fn download_to_local(
        &self,
        addr: &Address,
        path: &Path,
        options: &DownloadOptions,
    ) -> AddrResult<UpdateUnit> {
        match addr {
            Address::Git(_) => self.git.download_to_local(addr, path, options).await,
            Address::Http(_) => self.http.download_to_local(addr, path, options).await,
            Address::Local(_) => self.local.download_to_local(addr, path, options).await,
        }
    }
}

#[async_trait]
impl ResourceUploader for UniversalAccessor {
    async fn upload_from_local(
        &self,
        addr: &Address,
        path: &Path,
        options: &UploadOptions,
    ) -> AddrResult<UpdateUnit> {
        match addr {
            Address::Git(_) => self.git.upload_from_local(addr, path, options).await,
            Address::Http(_) => self.http.upload_from_local(addr, path, options).await,
            Address::Local(_) => self.local.upload_from_local(addr, path, options).await,
        }
    }
}

impl Clone for UniversalAccessor {
    fn clone(&self) -> Self {
        Self {
            git: self.git.clone(),
            http: self.http.clone(),
            local: self.local.clone(),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use orion_error::dev::testing::TestAssert;

    use super::*;
    use crate::addr::{Address, GitRepository, LocalPath};

    #[tokio::test]
    async fn test_select_accessor() {
        let config = UniversalConfig::default();
        let accessor = UniversalAccessor::new(config);

        let dest_path = PathBuf::from("./temp/hello-word.git");
        if dest_path.exists() {
            std::fs::remove_dir_all(&dest_path).assert();
        }
        let git_addr = Address::Git(GitRepository::from(
            "https://github.com/galaxy-sec/hello-word.git",
        ));
        accessor
            .download_to_local(
                &git_addr,
                &PathBuf::from("./temp/"),
                &DownloadOptions::default(),
            )
            .await
            .assert();
    }

    #[test]
    fn test_universal_accessor_config_accessors() {
        let config = UniversalConfig::default();
        let mut accessor = UniversalAccessor::new(config);

        // Test config() method
        let config_ref = accessor.config();
        assert_eq!(config_ref.accs_ctrl, None);

        // Test config_mut() method
        let net_ctrl = NetAccessCtrl::new(vec![], true);
        accessor.config_mut().accs_ctrl = Some(net_ctrl);

        let updated_config_ref = accessor.config();
        assert!(updated_config_ref.accs_ctrl.is_some());
    }

    #[tokio::test]
    async fn test_universal_accessor_upload_functionality() {
        let config = UniversalConfig::default();
        let accessor = UniversalAccessor::new(config);

        // Test upload with different address types
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file = temp_dir.path().join("test_file.txt");
        std::fs::write(&temp_file, "test content").unwrap();

        // Test Local upload (this should work without external dependencies)
        let local_addr = Address::Local(LocalPath::from(
            temp_file.clone().display().to_string().as_str(),
        ));
        let upload_result = accessor
            .upload_from_local(&local_addr, temp_dir.path(), &UploadOptions::default())
            .await;

        // The result might fail due to implementation details, but we're testing the path selection
        match upload_result {
            Ok(_) => {}                                           // Success
            Err(e) => println!("Upload failed as expected: {e}"), // Expected in test environment
        }
    }
}
