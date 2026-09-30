use crate::{
    addr::{
        AddrError, AddrReason, AddrResult, Address, HttpResource, access_ctrl::serv::NetAccessCtrl,
        accessor::client::create_http_client_by_ctrl, http::filename_of_url,
    },
    prelude::*,
    types::ResourceDownloader,
    update::{DownloadOptions, HttpMethod, UploadOptions},
};

use bytes::Bytes;
use futures_core::stream::Stream;
use getset::{Getters, WithSetters};
use http_body::{Frame, SizeHint};
use orion_error::prelude::SourceRawErr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;
use tracing::{debug, info, instrument};

use crate::types::ResourceUploader;
use std::time::Duration;
use tracing::warn;

/// 下载失败时的最大尝试次数（含首次）。
const DOWNLOAD_MAX_ATTEMPTS: u32 = 3;
/// 重试退避基数：第 n 次重试前等待 `DOWNLOAD_RETRY_BASE * 2^(n-1)`。
const DOWNLOAD_RETRY_BASE: Duration = Duration::from_millis(300);

/// 单次下载尝试的失败分类：区分「值得重试」与「重试无意义」。
enum DownloadAttempt {
    /// 网络抖动 / 5xx / 静默截断等，可重试。
    Retryable(AddrError),
    /// 4xx / 本地 IO 等确定性失败，重试无益。
    Fatal(AddrError),
}

/// 进度追踪流包装器
struct ProgressStream<R> {
    reader: ReaderStream<R>,
    progress_bar: indicatif::ProgressBar,
    uploaded_bytes: Arc<AtomicU64>,
    total_size: u64,
}

impl<R> ProgressStream<R>
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
{
    fn new(
        reader: R,
        progress_bar: indicatif::ProgressBar,
        uploaded_bytes: Arc<AtomicU64>,
        total_size: u64,
    ) -> Self {
        Self {
            reader: ReaderStream::new(reader),
            progress_bar,
            uploaded_bytes,
            total_size,
        }
    }
}

impl<R> http_body::Body for ProgressStream<R>
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
{
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match Pin::new(&mut self.reader).poll_next(cx) {
            Poll::Ready(Some(result)) => match result {
                Ok(bytes) => {
                    let n = bytes.len() as u64;
                    let current_pos = self.uploaded_bytes.fetch_add(n, Ordering::Relaxed) + n;
                    self.progress_bar.set_position(current_pos);
                    Poll::Ready(Some(Ok(Frame::data(bytes))))
                }
                Err(e) => Poll::Ready(Some(Err(e))),
            },
            Poll::Ready(None) => {
                // EOF reached
                self.progress_bar.set_position(self.total_size);
                self.uploaded_bytes
                    .store(self.total_size, Ordering::Relaxed);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        hint.set_exact(self.total_size);
        hint
    }
}

#[derive(Getters, Clone, Debug, WithSetters, Default)]
#[getset(get = "pub")]
pub struct HttpAccessor {
    #[getset(set_with = "pub")]
    ctrl: Option<NetAccessCtrl>,
}

impl HttpAccessor {
    #[instrument(
        target = "orion_variate::addr::http",
        skip(self, file_path),
        fields(
            file_path = %file_path.as_ref().display(),
            url = %addr.url(),
            method = ?method,
        ),
    )]
    pub async fn upload<P: AsRef<Path>>(
        &self,
        addr: &HttpResource,
        file_path: P,
        method: &HttpMethod,
    ) -> AddrResult<()> {
        use indicatif::{ProgressBar, ProgressStyle};
        let mut ctx = OperationContext::doing("upload url")
            .with_auto_log()
            .with_mod_path("addr/http");
        let addr = if let Some(direct_serv) = &self.ctrl {
            direct_serv.direct_http_addr(addr.clone())
        } else {
            addr.clone()
        };

        let client =
            create_http_client_by_ctrl(self.ctrl().clone().and_then(|x| x.direct_http_ctrl(&addr)))
                .with_context(&ctx)?;
        let file_name = filename_of_url(addr.url()).unwrap_or_else(|| "file.bin".to_string());
        ctx.record("local file", file_path.as_ref().display());
        ctx.record("url ", addr.url().as_str());
        ctx.record("file", file_name.as_str());

        ctx.info("upload start...");

        // 异步打开文件并获取大小
        let file = tokio::fs::File::open(&file_path)
            .await
            .source_raw_err(AddrReason::data_error(), "")
            .with_context(&ctx)?;
        let metadata = file
            .metadata()
            .await
            .source_raw_err(AddrReason::data_error(), "")
            .with_context(&ctx)?;
        let content_len = metadata.len();

        // 创建原子计数器用于进度追踪
        let uploaded_bytes = Arc::new(AtomicU64::new(0));

        // 创建进度条
        let pb = ProgressBar::new(content_len);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})").source_raw_err(AddrReason::logic_error(), "")?
            .progress_chars("#>-"));

        // 创建进度追踪流
        let progress_stream =
            ProgressStream::new(file, pb.clone(), uploaded_bytes.clone(), content_len);

        // 创建请求
        let request = match method {
            HttpMethod::Post => {
                // Post方法 - 使用multipart表单
                let body = reqwest::Body::wrap(progress_stream);
                let part = reqwest::multipart::Part::stream(body).file_name(file_name.clone());
                let form = reqwest::multipart::Form::new().part("file", part);
                let mut request = client.post(addr.url()).multipart(form);

                // 添加认证信息
                if let (Some(u), Some(p)) = (addr.username(), addr.password()) {
                    request = request.basic_auth(u, Some(p));
                }
                request
            }
            HttpMethod::Put => {
                // PUT方法 - 直接流式上传
                let body = reqwest::Body::wrap(progress_stream);
                let mut request = client.put(addr.url()).body(body);

                // 添加认证信息
                if let (Some(u), Some(p)) = (addr.username(), addr.password()) {
                    request = request.basic_auth(u, Some(p));
                }
                request
            }
            _ => {
                return Err(AddrReason::resource_error()
                    .to_err()
                    .doing(format!("Unsupported HTTP method: {method}")));
            }
        };

        // 设置初始进度
        pb.set_position(0);

        ctx.debug("sending http upload request");

        // 发送请求 - 进度会在流读取时自动更新
        let response = request
            .send()
            .await
            .source_raw_err(AddrReason::resource_error(), "")
            .with_context(&ctx)?;
        response
            .error_for_status()
            .source_raw_err(AddrReason::resource_error(), "")
            .with_context(&ctx)?;

        pb.finish_with_message("上传完成");
        ctx.info("upload completed");
        ctx.mark_suc();
        Ok(())
    }

    #[instrument(
        target = "orion_variate::addr::http",
        skip(self, dest_path),
        fields(
            url = %addr.url(),
            dest_path = %dest_path.display(),
            cache_reuse = options.reuse_cache(),
        ),
        err(Debug),
    )]
    pub async fn download(
        &self,
        addr: &HttpResource,
        dest_path: &Path,
        options: &DownloadOptions,
    ) -> AddrResult<PathBuf> {
        let addr = if let Some(direct_serv) = &self.ctrl {
            direct_serv.direct_http_addr(addr.clone())
        } else {
            addr.clone()
        };

        if dest_path.exists() && options.reuse_cache() {
            info!(
                target: "orion_variate::addr::http",
                path = %dest_path.display(),
                "file already exists, skipping download due to reuse_cache"
            );
            return Ok(dest_path.to_path_buf());
        }
        let mut ctx = OperationContext::doing("download url")
            .with_auto_log()
            .with_mod_path("addr/http");
        ctx.record("url", addr.url().as_str());
        ctx.record("local", dest_path.display().to_string());

        // 先下到**同目录**临时文件，成功后 rename 原子替换：
        // 任何失败都不会在 `dest_path` 上留半包（也不会提前删掉已存在的旧文件），
        // 因此 `reuse_cache` 跳过时不会把半包当成有效缓存。
        //
        // 注：`reuse_cache` 跳过时不再做任何校验；旧版本（< 0.8.3）遗留的截断文件
        // 仍然会被当成有效缓存，需要手动删除（或改用 `UpdateScope::RemoteCache` 强刷）。
        let part_path = part_path(dest_path);
        let mut last_err: Option<AddrError> = None;
        for attempt in 1..=DOWNLOAD_MAX_ATTEMPTS {
            remove_file_if_exists(&part_path);
            match self.download_once(&addr, &part_path, &mut ctx).await {
                Ok(_) => {
                    if let Err(e) = tokio::fs::rename(&part_path, dest_path).await {
                        // rename 失败也别留临时文件；原始目标文件保持不变。
                        remove_file_if_exists(&part_path);
                        return Err(AddrReason::system_error()
                            .to_err()
                            .doing(format!(
                                "rename {} -> {}: {e}",
                                part_path.display(),
                                dest_path.display()
                            ))
                            .with_context(&ctx));
                    }
                    debug!(
                        target: "orion_variate::addr::http",
                        path = %dest_path.display(),
                        "download completed"
                    );
                    ctx.mark_suc();
                    return Ok(dest_path.to_path_buf());
                }
                Err(DownloadAttempt::Fatal(e)) => {
                    remove_file_if_exists(&part_path);
                    return Err(e);
                }
                Err(DownloadAttempt::Retryable(e)) => {
                    remove_file_if_exists(&part_path);
                    if attempt < DOWNLOAD_MAX_ATTEMPTS {
                        let backoff = DOWNLOAD_RETRY_BASE * 2u32.pow(attempt - 1);
                        warn!(
                            target: "orion_variate::addr::http",
                            url = %addr.url(),
                            attempt,
                            max = DOWNLOAD_MAX_ATTEMPTS,
                            backoff_ms = backoff.as_millis() as u64,
                            error = %e,
                            "download attempt failed; retrying"
                        );
                        tokio::time::sleep(backoff).await;
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(AddrReason::RetryExhausted {
            attempts: DOWNLOAD_MAX_ATTEMPTS,
            last_error: last_err.map(|e| e.to_string()).unwrap_or_default(),
        }
        .to_err()
        .doing(format!("download {}", addr.url()))
        .with_context(&ctx))
    }

    /// 单次下载：请求 → 流式写入 `part_path` → 长度校验。
    ///
    /// 返回已下载字节数；失败按 [`DownloadAttempt`] 分类，调用方决定是否重试。
    async fn download_once(
        &self,
        addr: &HttpResource,
        part_path: &Path,
        ctx: &mut OperationContext,
    ) -> Result<u64, DownloadAttempt> {
        use indicatif::{ProgressBar, ProgressStyle};
        use tokio::io::AsyncWriteExt;

        let client =
            create_http_client_by_ctrl(self.ctrl().clone().and_then(|x| x.direct_http_ctrl(addr)))
                .with_context(&*ctx)
                .map_err(DownloadAttempt::Fatal)?;
        let mut request = client.get(addr.url());
        if let (Some(u), Some(p)) = (addr.username(), addr.password()) {
            request = request.basic_auth(u, Some(p));
        }

        let mut response = request
            .send()
            .await
            .source_raw_err(AddrReason::resource_error(), "")
            .with_context(&*ctx)
            .map_err(DownloadAttempt::Retryable)?;

        let status = response.status();
        if !status.is_success() {
            let err = AddrReason::resource_error()
                .to_err()
                .doing(format!("HTTP request failed: {status}"))
                .with_context(&*ctx);
            // 4xx 是确定性失败（重试无意义）；5xx 可能是瞬时故障。
            return Err(if status.is_client_error() {
                DownloadAttempt::Fatal(err)
            } else {
                DownloadAttempt::Retryable(err)
            });
        }

        // `Content-Length` 缺失（如 chunked）时为 `None`，此时不做长度校验。
        let total_size = response.content_length();

        let mut file = tokio::fs::File::create(part_path)
            .await
            .source_raw_err(AddrReason::core_conf(), "")
            .with_context(&*ctx)
            .map_err(DownloadAttempt::Fatal)?;

        // 创建进度条
        let pb = ProgressBar::new(total_size.unwrap_or(0));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})").source_raw_err(AddrReason::logic_error(), "")
            .map_err(DownloadAttempt::Fatal)?
            .progress_chars("#>-"));

        let mut downloaded: u64 = 0;

        debug!(
            target: "orion_variate::addr::http",
            url = %addr.url(),
            total_size = ?total_size,
            "starting download stream"
        );
        loop {
            let chunk = response
                .chunk()
                .await
                .source_raw_err(AddrReason::data_error(), "")
                .with_context(&*ctx)
                .map_err(DownloadAttempt::Retryable)?;
            let Some(chunk) = chunk else { break };
            file.write_all(&chunk)
                .await
                .source_raw_err(AddrReason::system_error(), "")
                .with_context(&*ctx)
                .map_err(DownloadAttempt::Fatal)?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }
        file.flush()
            .await
            .source_raw_err(AddrReason::system_error(), "")
            .with_context(&*ctx)
            .map_err(DownloadAttempt::Fatal)?;
        drop(file);

        // 长度校验：服务端给出了 `Content-Length` 时，收到的字节数必须完全一致，
        // 否则视为截断（防服务端提前关连接被当成成功）。
        if let Some((got, want)) = length_mismatch(downloaded, total_size) {
            pb.abandon_with_message(format!("截断 {got}/{want}"));
            return Err(DownloadAttempt::Retryable(
                AddrReason::data_error()
                    .to_err()
                    .doing(format!("download truncated: {got} of {want} bytes"))
                    .with_context(&*ctx),
            ));
        }

        pb.finish_with_message("下载完成");
        Ok(downloaded)
    }
}

/// 下载临时文件路径：同目录下的 `<name>.part`（保证 rename 是同文件系统内的原子操作）。
fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// 长度校验：服务端给出 `total`（`Content-Length`）且与实收字节不一致时，
/// 返回 `(实收, 应为)`；`total` 为 `None`（chunked 等未知长度）时不校验。
fn length_mismatch(downloaded: u64, total: Option<u64>) -> Option<(u64, u64)> {
    match total {
        Some(n) if downloaded != n => Some((downloaded, n)),
        _ => None,
    }
}

/// 尽力删除文件；不存在或删除失败都不报错（调用方只关心“别留半包”）。
fn remove_file_if_exists(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => debug!(
            target: "orion_variate::addr::http",
            path = %path.display(),
            error = %e,
            "failed to remove temp file"
        ),
    }
}

#[async_trait]
impl ResourceDownloader for HttpAccessor {
    #[instrument(
        target = "orion_variate::addr::http",
        skip(self, dest_dir, options),
        fields(
            addr = %addr,
            dest_dir = %dest_dir.display(),
        ),
    )]
    async fn download_to_local(
        &self,
        addr: &Address,
        dest_dir: &Path,
        options: &DownloadOptions,
    ) -> AddrResult<UpdateUnit> {
        match addr {
            Address::Http(http) => {
                let target_path = if dest_dir.is_dir() {
                    let file = filename_of_url(http.url());
                    &dest_dir.join(file.unwrap_or("file.tmp".into()))
                } else {
                    dest_dir
                };
                Ok(UpdateUnit::from(
                    self.download(http, target_path, options).await?,
                ))
            }
            _ => Err(AddrReason::Brief(format!("addr type error {addr}")).to_err()),
        }
    }
}

#[async_trait]
impl ResourceUploader for HttpAccessor {
    #[instrument(
        target = "orion_variate::addr::http",
        skip(self, path, options),
        fields(
            addr = %addr,
            path = %path.display(),
        ),
    )]
    async fn upload_from_local(
        &self,
        addr: &Address,
        path: &Path,
        options: &UploadOptions,
    ) -> AddrResult<UpdateUnit> {
        if !path.exists() {
            return Err(AddrReason::resource_error()
                .to_err()
                .doing("path not exist"));
        }
        match addr {
            Address::Http(http) => {
                self.upload(http, path, options.http_method()).await?;
                /*
                if path.is_file() {
                    std::fs::remove_file(path).source_raw_err(AddrReason::resource_error(), "")?;
                } else {
                    std::fs::remove_dir_all(path).source_raw_err(AddrReason::resource_error(), "")?;
                }
                */
                Ok(UpdateUnit::from(path.to_path_buf()))
            }
            _ => Err(AddrReason::Brief(format!("addr type error {addr}")).to_err()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        addr::{
            AddrResult,
            access_ctrl::{AuthConfig, Rule},
        },
        tools::test_init,
        update::DownloadOptions,
    };

    use super::*;
    use mockito::Matcher;
    use orion_error::dev::testing::TestAssertWithMsg;
    use orion_infra::path::ensure_path;

    #[tokio::test(flavor = "current_thread")]
    async fn test_http_auth_download_no() -> AddrResult<()> {
        // 1. 配置模拟服务器
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", "/wpflow.txt")
            .match_header("Authorization", Matcher::Exact("Basic Z2VuZXJpYy0xNzQ3NTM1OTc3NjMyOjViMmM5ZTliN2YxMTFhZjUyZjAzNzVjMWZkOWQzNWNkNGQwZGFiYzM=".to_string()))
            .with_status(200)
            .with_header("content-type", "text/html; charset=UTF-8")
            .with_body("download success")
            .create();

        // 2. 执行下载
        let temp_dir = PathBuf::from("./tests/temp");
        let test_file = temp_dir.join("wpflow.txt");
        if test_file.exists() {
            std::fs::remove_file(&test_file).source_raw_err(AddrReason::resource_error(), "")?;
        }
        let http_addr = HttpResource::from(format!("{}/wpflow.txt", server.url()))
            .with_credentials(
                "generic-1747535977632",
                "5b2c9e9b7f111af52f0375c1fd9d35cd4d0dabc3",
            );

        let http_accessor = HttpAccessor::default();
        http_accessor
            .download_to_local(
                &Address::from(http_addr),
                &temp_dir,
                &DownloadOptions::for_test(),
            )
            .await?;

        // 3. 验证结果
        assert!(test_file.exists());
        mock.assert();
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_http_auth_download_with_redirect() -> AddrResult<()> {
        test_init();
        // 1. 配置模拟服务器
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", "/success.txt")
            .match_header("Authorization", Matcher::Exact("Basic Z2VuZXJpYy0xNzQ3NTM1OTc3NjMyOjViMmM5ZTliN2YxMTFhZjUyZjAzNzVjMWZkOWQzNWNkNGQwZGFiYzM=".to_string()))
            .with_status(200)
            .with_header("content-type", "text/html; charset=UTF-8")
            .with_body("download success")
            .create();

        // 2. 执行下载
        let temp_dir = PathBuf::from("./tests/temp");
        ensure_path(&temp_dir).assert("path");
        let test_file = temp_dir.join("unkonw.txt");
        if test_file.exists() {
            std::fs::remove_file(&test_file).source_raw_err(AddrReason::resource_error(), "")?;
        }
        let redirect = NetAccessCtrl::from_rule(
            Rule::new(
                format!("{}/unkonw*", server.url()),
                format!("{}/success", server.url()),
            ),
            Some(AuthConfig::new(
                "generic-1747535977632",
                "5b2c9e9b7f111af52f0375c1fd9d35cd4d0dabc3",
            )),
            None,
        );
        let http_addr = HttpResource::from(format!("{}/unkonw.txt", server.url()));

        let http_accessor = HttpAccessor::default().with_ctrl(Some(redirect));
        http_accessor
            .download_to_local(
                &Address::from(http_addr),
                &temp_dir,
                &DownloadOptions::for_test(),
            )
            .await?;

        // 3. 验证结果
        assert!(test_file.exists());
        mock.assert();
        Ok(())
    }
    #[ignore = "need more time"]
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_addr() -> AddrResult<()> {
        let path = PathBuf::from("/tmp");
        let addr = HttpResource::from(
            "https://dy-sec-generic.pkg.coding.net/sec-hub/generic/warp-flow/wpflow?version=1.0.89-alpha",
        )
        .with_credentials(
                    "generic-1747535977632",
                    "5b2c9e9b7f111af52f0375c1fd9d35cd4d0dabc3",
                );
        let http_accessor = HttpAccessor::default();
        http_accessor
            .download_to_local(&Address::from(addr), &path, &DownloadOptions::for_test())
            .await?;
        Ok(())
    }

    #[test]
    fn test_part_path_appends_suffix_in_same_dir() {
        let p = part_path(Path::new("/tmp/foo/bar.tar.gz"));
        assert_eq!(p, PathBuf::from("/tmp/foo/bar.tar.gz.part"));
        // 与目标同目录（保证 rename 是同文件系统内的原子操作）
        assert_eq!(p.parent(), Some(Path::new("/tmp/foo")));
    }

    /// 服务端 5xx：应重试到上限，且**不在目标路径留半包**。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_retries_then_no_partial_left() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/retry.bin")
            .with_status(503)
            .expect(DOWNLOAD_MAX_ATTEMPTS as usize)
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("retry.bin");
        let accessor = HttpAccessor::default();

        let res = accessor
            .download(
                &HttpResource::from(format!("{}/retry.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await;

        assert!(res.is_err(), "5xx should fail after retries");
        assert!(!dest.exists(), "no partial file at dest");
        assert!(!part_path(&dest).exists(), "no temp file left");
        mock.assert(); // 恰好重试到上限
        Ok(())
    }

    /// 下载失败时**不能删掉已存在的旧文件**（旧实现的 `remove_file` 会先删）。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_failure_keeps_existing_file() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server.mock("GET", "/keep.bin").with_status(500).create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("keep.bin");
        std::fs::write(&dest, "old-content").source_raw_err(AddrReason::resource_error(), "")?;

        let res = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/keep.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await;

        assert!(res.is_err());
        assert_eq!(
            std::fs::read_to_string(&dest).source_raw_err(AddrReason::resource_error(), "")?,
            "old-content",
            "existing file must be untouched on failure"
        );
        assert!(!part_path(&dest).exists());
        Ok(())
    }

    /// 成功：写临时文件 → rename 替换；不留 `.part`。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_success_replaces_existing() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/fresh.bin")
            .with_status(200)
            .with_body("fresh-content")
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("fresh.bin");
        std::fs::write(&dest, "old-content").source_raw_err(AddrReason::resource_error(), "")?;

        let out = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/fresh.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await?;

        assert_eq!(out, dest);
        assert_eq!(
            std::fs::read_to_string(&dest).source_raw_err(AddrReason::resource_error(), "")?,
            "fresh-content"
        );
        assert!(!part_path(&dest).exists(), "temp file must be gone");
        mock.assert();
        Ok(())
    }

    /// 声明长度与实际收到的字节不符（截断）必须报错。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_truncated_body_is_error() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/trunc.bin")
            .with_status(200)
            .with_header("content-length", "999999")
            .with_body("short")
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("trunc.bin");

        let res = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/trunc.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await;

        assert!(res.is_err(), "truncated body must be an error");
        assert!(!dest.exists(), "no partial file at dest");
        assert!(!part_path(&dest).exists());
        Ok(())
    }

    /// `reuse_cache` 且目标已存在：直接跳过，不发请求。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_skips_when_reuse_cache() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", "/cached.bin").expect(0).create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("cached.bin");
        std::fs::write(&dest, "cached").source_raw_err(AddrReason::resource_error(), "")?;

        let out = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/cached.bin", server.url())),
                &dest,
                &DownloadOptions::default(), // reuse_cache = true
            )
            .await?;

        assert_eq!(out, dest);
        assert_eq!(
            std::fs::read_to_string(&dest).source_raw_err(AddrReason::resource_error(), "")?,
            "cached"
        );
        mock.assert(); // 未发起请求
        Ok(())
    }

    #[test]
    fn test_length_mismatch_matrix() {
        // 一致 → 不报
        assert_eq!(length_mismatch(9, Some(9)), None);
        // 截断 / 超长 → 报（实收, 应为）
        assert_eq!(length_mismatch(5, Some(9)), Some((5, 9)));
        assert_eq!(length_mismatch(10, Some(9)), Some((10, 9)));
        // 声明为 0 也是“已知长度”，必须恰好 0
        assert_eq!(length_mismatch(0, Some(0)), None);
        assert_eq!(length_mismatch(3, Some(0)), Some((3, 0)));
        // 未知长度（chunked）不校验
        assert_eq!(length_mismatch(0, None), None);
        assert_eq!(length_mismatch(1234, None), None);
    }

    /// 4xx 是确定性失败：不重试（只发一次请求）。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_4xx_is_not_retried() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/missing.bin")
            .with_status(404)
            .expect(1)
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("missing.bin");

        let res = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/missing.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await;

        assert!(res.is_err());
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
        mock.assert(); // 只请求一次
        Ok(())
    }

    /// 重试耗尽后返回 `AddrReason::RetryExhausted`（保留尝试次数与最后一次错误）。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_exhaustion_reports_retry_reason() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server.mock("GET", "/down.bin").with_status(502).create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("down.bin");

        let err = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/down.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await
            .expect_err("5xx should fail after retries");

        match err.reason() {
            AddrReason::RetryExhausted {
                attempts,
                last_error,
            } => {
                assert_eq!(*attempts, DOWNLOAD_MAX_ATTEMPTS);
                assert!(!last_error.is_empty(), "last error should be recorded");
            }
            other => panic!("expected RetryExhausted, got {other:?}"),
        }
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
        Ok(())
    }

    /// 无 `Content-Length`（chunked）：不校验长度，正常成功。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_chunked_body_is_ok() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/chunked.bin")
            .with_status(200)
            .with_chunked_body(|w| {
                w.write_all(b"chunk-1")?;
                w.write_all(b"chunk-2")?;
                Ok(())
            })
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("chunked.bin");

        HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/chunked.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await?;

        assert_eq!(
            std::fs::read_to_string(&dest).source_raw_err(AddrReason::resource_error(), "")?,
            "chunk-1chunk-2"
        );
        Ok(())
    }

    /// 空文件（`Content-Length: 0`）：合法，不算截断。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_empty_body_is_ok() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/empty.bin")
            .with_status(200)
            .with_body("")
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("empty.bin");

        HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/empty.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await?;

        assert_eq!(
            std::fs::metadata(&dest)
                .source_raw_err(AddrReason::resource_error(), "")?
                .len(),
            0
        );
        Ok(())
    }

    /// 本地 IO 失败（目标目录不存在）：不重试。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_io_error_is_not_retried() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/io.bin")
            .with_status(200)
            .with_body("x")
            .expect(1)
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        // 父目录不存在 → 写临时文件失败
        let dest = dir.path().join("no-such-dir").join("io.bin");

        let res = HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/io.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await;

        assert!(res.is_err());
        assert!(!dest.exists());
        mock.assert(); // 只请求一次（IO 错误不重试）
        Ok(())
    }

    /// 上次中断遗留的 `.part` 会在下次下载时被清理，不影响结果。
    #[tokio::test(flavor = "current_thread")]
    async fn test_http_download_cleans_stale_part_file() -> AddrResult<()> {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/resume.bin")
            .with_status(200)
            .with_body("good")
            .create();

        let dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let dest = dir.path().join("resume.bin");
        std::fs::write(part_path(&dest), "stale-half")
            .source_raw_err(AddrReason::resource_error(), "")?;

        HttpAccessor::default()
            .download(
                &HttpResource::from(format!("{}/resume.bin", server.url())),
                &dest,
                &DownloadOptions::for_test(),
            )
            .await?;

        assert_eq!(
            std::fs::read_to_string(&dest).source_raw_err(AddrReason::resource_error(), "")?,
            "good"
        );
        assert!(!part_path(&dest).exists());
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_http_upload_post() -> AddrResult<()> {
        // 1. 配置模拟服务器
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("POST", "/upload")
            .match_header("content-type", Matcher::Regex("multipart/form-data.*".to_string()))
            .match_header("Authorization", Matcher::Exact("Basic Z2VuZXJpYy0xNzQ3NTM1OTc3NjMyOjViMmM5ZTliN2YxMTFhZjUyZjAzNzVjMWZkOWQzNWNkNGQwZGFiYzM=".to_string()))
            .with_status(200)
            .with_body("upload success")
            .create();

        // 2. 创建临时测试文件
        let temp_dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let file_path = temp_dir.path().join("test.txt");
        tokio::fs::write(&file_path, "test content")
            .await
            .source_raw_err(AddrReason::system_error(), "")?;

        // 3. 执行上传
        let http_addr = HttpResource::from(format!("{}/upload", server.url())).with_credentials(
            "generic-1747535977632",
            "5b2c9e9b7f111af52f0375c1fd9d35cd4d0dabc3",
        );
        let http_accessor = HttpAccessor::default();

        http_accessor
            .upload(&http_addr, &file_path, &HttpMethod::Post)
            .await?;

        // 4. 验证结果
        mock.assert();
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_http_upload_put() -> AddrResult<()> {
        // 1. 配置模拟服务器
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("PUT", "/upload_put")
            .match_header("Authorization", Matcher::Exact("Basic Z2VuZXJpYy0xNzQ3NTM1OTc3NjMyOjViMmM5ZTliN2YxMTFhZjUyZjAzNzVjMWZkOWQzNWNkNGQwZGFiYzM=".to_string()))
            .with_status(200)
            .with_body("upload success")
            .create();

        // 2. 创建临时测试文件
        let temp_dir = tempfile::tempdir().source_raw_err(AddrReason::resource_error(), "")?;
        let file_path = temp_dir.path().join("test_put.txt");
        tokio::fs::write(&file_path, "test put content")
            .await
            .source_raw_err(AddrReason::system_error(), "")?;

        // 3. 执行上传
        let http_addr = HttpResource::from(format!("{}/upload_put", server.url()))
            .with_credentials(
                "generic-1747535977632",
                "5b2c9e9b7f111af52f0375c1fd9d35cd4d0dabc3",
            );
        let http_accessor = HttpAccessor::default();

        http_accessor
            .upload(&http_addr, &file_path, &HttpMethod::Put)
            .await?;

        // 4. 验证结果
        mock.assert();
        Ok(())
    }
}
