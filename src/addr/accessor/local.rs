use crate::addr::{AddrReason, AddrResult, Address};
use crate::prelude::*;
use crate::types::ResourceDownloader;
use crate::update::{DownloadOptions, UploadOptions};
use contracts::debug_requires;
use fs_extra::dir::CopyOptions;
use orion_error::UvsFrom;
use orion_error::traits_ext::{ContextRecord, ToStructError};

use crate::types::ResourceUploader;

#[derive(Getters, Clone, Debug, Default)]
pub struct LocalAccessor {}

#[async_trait]
impl ResourceDownloader for LocalAccessor {
    //#[debug_ensures(matches!(*result, Ok(v) if v.exists()), "path not exists")]
    async fn download_to_local(
        &self,
        addr: &Address,
        path: &Path,
        up_options: &DownloadOptions,
    ) -> AddrResult<UpdateUnit> {
        let addr = match addr {
            Address::Local(addr) => addr,
            _ => return Err(AddrReason::Brief(format!("addr type error {addr}")).to_err()),
        };
        let mut ctx = OperationContext::doing("update local addr")
            .with_auto_log()
            .with_mod_path("addr/local");
        ctx.record("src", addr.path().as_str());
        ctx.record("dst", path.display().to_string());
        let src = PathBuf::from(addr.path().as_str());
        let options = CopyOptions::new().overwrite(true); // 默认选项

        std::fs::create_dir_all(path).owe_res()?;
        let name = path_file_name(&src)?;
        let dst = path.join(name);
        let _dst_copy = dst.clone();

        if src.is_file() {
            std::fs::copy(&src, &dst).owe_res()?;
        } else if dst.exists() && up_options.reuse_cache() {
            info!(
                target : "spec/addr/local",
                "ignore update {} to {} !", src.display(),_dst_copy.display()
            );
        } else {
            fs_extra::dir::copy(&src, path, &options)
                .owe_data()
                .with_context(&ctx)?;
        }
        ctx.mark_suc();
        Ok(UpdateUnit::from(dst))
    }

    async fn download_rename(
        &self,
        addr: &Address,
        path: &Path,
        name: &str,
        options: &DownloadOptions,
    ) -> AddrResult<UpdateUnit> {
        let target = self.download_to_local(addr, path, options).await?;
        Ok(UpdateUnit::from(rename_path(target.position(), name)?))
    }
}

#[async_trait]
impl ResourceUploader for LocalAccessor {
    async fn upload_from_local(
        &self,
        addr: &Address,
        path: &Path,
        options: &UploadOptions,
    ) -> AddrResult<UpdateUnit> {
        let _ = options; // 使用options参数，为后续实现支持上传配置
        let addr = match addr {
            Address::Local(addr) => addr,
            _ => return Err(AddrReason::Brief(format!("addr type error {addr}")).to_err()),
        };
        if !path.exists() {
            return Err(AddrReason::from_res().to_err().doing("path not exist"));
        }
        if path.is_file() {
            let file_name = path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("UNKNOW");
            let target_path = Path::new(addr.path()).join(file_name);
            std::fs::copy(path, target_path).owe_res()?;
            std::fs::remove_file(path).owe_res()?;
        } else {
            let copy_options = CopyOptions::new().overwrite(true).copy_inside(true);
            fs_extra::dir::copy(path, addr.path(), &copy_options).owe_res()?;
            std::fs::remove_dir_all(path).owe_res()?;
        }
        Ok(UpdateUnit::from(path.to_path_buf()))
    }
}

pub fn path_file_name(path: &Path) -> AddrResult<String> {
    let file_name = path.file_name().and_then(|f| f.to_str()).ok_or(
        AddrReason::from_conf()
            .to_err()
            .doing("get file_name error"),
    )?;
    Ok(file_name.to_string())
}
#[debug_requires(local.exists(), "local need exists")]
pub fn rename_path(local: &Path, name: &str) -> AddrResult<PathBuf> {
    let mut ctx = OperationContext::doing("rename path")
        .with_auto_log()
        .with_mod_path("addr/local");
    let dst_path = local
        .parent()
        .map(|x| x.join(name))
        .ok_or(AddrReason::from_conf().to_err().doing("bad path"))?;

    let _dst_copy = dst_path.clone();
    if dst_path.exists() {
        if dst_path == local {
            ctx.mark_suc();
            return Ok(dst_path.clone());
        }
        if dst_path.is_dir() {
            std::fs::remove_dir_all(&dst_path)
                .owe_res()
                .with_context(&dst_path)
                .doing("remove dst")?;
        } else {
            std::fs::remove_file(&dst_path)
                .owe_res()
                .with_context(&dst_path)
                .doing("remove dst")?;
        }
    }
    ctx.record("new path", dst_path.display().to_string());
    std::fs::rename(local, &dst_path)
        .owe_conf()
        .with_context(&ctx)?;
    ctx.mark_suc();
    Ok(dst_path)
}

#[cfg(test)]
mod tests {
    use crate::{
        addr::{AddrResult, LocalPath},
        tools::test_init,
        update::DownloadOptions,
    };

    use super::*;
    use orion_error::testcase::TestAssert;
    use orion_infra::path::ensure_path;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_local() -> AddrResult<()> {
        test_init();
        let temp_path = PathBuf::from("./tests/temp/local");
        if temp_path.exists() {
            std::fs::remove_dir_all(&temp_path).owe_conf()?;
        }
        std::fs::create_dir_all(&temp_path).owe_conf()?;
        let local = LocalPath::from("./tests/data/sys-1");
        let addr_type = Address::Local(local.clone());
        LocalAccessor::default()
            .download_rename(
                &addr_type,
                &temp_path,
                "sys-2",
                &DownloadOptions::for_test(),
            )
            .await?;
        LocalAccessor::default()
            .upload_from_local(
                &Address::from(LocalPath::from("./tests/temp/local/sys-3")),
                &temp_path.join("sys-2"),
                &UploadOptions::new(),
            )
            .await?;

        assert!(std::fs::exists(temp_path.join("sys-3")).owe_conf()?);
        //assert!(std::fs::exists(temp_path.join("sys-1")).owe_conf()?);
        Ok(())
    }

    #[test]
    fn test_rename_path_file_new_model() -> AddrResult<()> {
        // 创建临时目录
        let temp_dir = tempdir().assert();
        let src_path = temp_dir.path().join("source.txt");
        std::fs::write(&src_path, "test content").assert();

        // 执行重命名（目标不存在）
        let renamed = rename_path(&src_path, "renamed.txt").assert();

        // 验证结果
        assert!(renamed.exists());
        assert!(!src_path.exists());
        assert_eq!(renamed.file_name().unwrap(), "renamed.txt");
        Ok(())
    }

    #[test]
    fn test_rename_path_file_overwrite_existing_file() -> AddrResult<()> {
        // 创建临时目录
        let temp_dir = tempdir().assert();
        let src_path = temp_dir.path().join("source.txt");
        let target_path = temp_dir.path().join("existing.txt");
        std::fs::write(&src_path, "source content").assert();
        std::fs::write(&target_path, "existing content").assert();

        // 执行重命名（覆盖现有文件）
        let renamed = rename_path(&src_path, "existing.txt").assert();

        // 验证结果
        assert!(renamed.exists());
        assert!(!src_path.exists());
        assert_eq!(std::fs::read_to_string(&renamed).assert(), "source content"); // 应覆盖原有内容
        Ok(())
    }

    #[test]
    fn test_rename_path_dir_new_model() -> AddrResult<()> {
        // 创建临时目录
        let temp_dir = PathBuf::from("./tests/temp/rename_test");
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir).assert();
        }
        std::fs::create_dir_all(&temp_dir).assert();

        let src_dir = temp_dir.join("source_dir");
        let new_dir = temp_dir.join("renamed_dir");
        std::fs::create_dir(&src_dir).assert();
        std::fs::write(src_dir.join("file.txt"), "test").assert();

        // 执行重命名（目标不存在）
        let renamed = rename_path(&src_dir, "renamed_dir").assert();

        // 验证结果
        assert!(renamed.exists());
        assert!(renamed.join("file.txt").exists());
        assert!(!src_dir.exists());
        assert!(new_dir.exists());
        Ok(())
    }

    #[test]
    fn test_rename_path_dir_overwrite_existing_dir() -> AddrResult<()> {
        // 创建临时目录
        let temp_dir = tempdir().assert();
        let src_dir = temp_dir.path().join("source_dir");
        let target_dir = temp_dir.path().join("existing_dir");
        std::fs::create_dir(&src_dir).assert();
        std::fs::create_dir(&target_dir).assert();
        std::fs::write(src_dir.join("source_file.txt"), "source").assert();
        std::fs::write(target_dir.join("existing_file.txt"), "existing").assert();

        // 执行重命名（覆盖现有目录）
        let renamed = rename_path(&src_dir, "existing_dir")?;

        // 验证结果
        assert!(renamed.exists());
        assert!(renamed.join("source_file.txt").exists()); // 源目录内容应保留
        assert!(!renamed.join("existing_file.txt").exists()); // 原目标目录应被删除
        assert!(!src_dir.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_upload_file_from_local() -> AddrResult<()> {
        //let target = tempdir().assert();
        //let target_dir = target.path();
        let source = tempdir().assert();
        let source_dir = source.path();

        let file_path = source_dir.join("file.txt");
        std::fs::write(&file_path, "source").assert();
        //let local_addr = LocalPath::from(target_dir.to_str().unwrap_or("~/temp"));
        let local_addr = LocalPath::from("/tmp/test-upload-dir");
        ensure_path(PathBuf::from(local_addr.path()).as_path()).assert();
        let addr_type = Address::Local(local_addr.clone());
        LocalAccessor::default()
            .upload_from_local(&addr_type, file_path.as_path(), &UploadOptions::new())
            .await?;

        assert!(PathBuf::from(local_addr.path()).join("file.txt").exists());
        assert!(!file_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_upload_dir_from_local() -> AddrResult<()> {
        let target = tempdir().assert();
        let target_dir = target.path();
        let source = tempdir().assert();
        let source_dir = source.path();

        let version_1 = target_dir.join("version_1");
        let version_2 = source_dir.join("version_2");
        std::fs::create_dir_all(&version_1).assert();
        std::fs::create_dir_all(&version_2).assert();

        std::fs::write(version_2.join("version_2.txt"), "version_2").assert();

        let local_addr = LocalPath::from(version_1.to_str().unwrap_or("~/temp"));
        let addr_type = Address::Local(local_addr.clone());
        LocalAccessor::default()
            .upload_from_local(&addr_type, &version_2, &UploadOptions::new())
            .await?;

        assert!(version_1.join("version_2").exists());
        assert!(version_1.join("version_2").join("version_2.txt").exists());
        assert!(!version_2.exists());
        Ok(())
    }
}
