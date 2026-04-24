use crate::{
    addr::{
        AddrReason, AddrResult, Address, HttpResource, access_ctrl::serv::NetAccessCtrl,
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
use orion_error::UvsFrom;
use orion_error::traits_ext::{ContextRecord, ToStructError};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;
use tracing::{debug, info, instrument};

use crate::types::ResourceUploader;

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
        ctx.record("local file", file_path.as_ref());
        ctx.record("url ", addr.url().as_str());
        ctx.record("file", file_name.as_str());

        ctx.info("upload start...");

        // 异步打开文件并获取大小
        let file = tokio::fs::File::open(&file_path)
            .await
            .owe_data()
            .with_context(&ctx)?;
        let metadata = file.metadata().await.owe_data().with_context(&ctx)?;
        let content_len = metadata.len();

        // 创建原子计数器用于进度追踪
        let uploaded_bytes = Arc::new(AtomicU64::new(0));

        // 创建进度条
        let pb = ProgressBar::new(content_len);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})").owe_logic()?
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
                return Err(AddrReason::from_res()
                    .to_err()
                    .doing(format!("Unsupported HTTP method: {method}")));
            }
        };

        // 设置初始进度
        pb.set_position(0);

        ctx.debug("sending http upload request");

        // 发送请求 - 进度会在流读取时自动更新
        let response = request.send().await.owe_res().with_context(&ctx)?;
        response.error_for_status().owe_res().with_context(&ctx)?;

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
        use indicatif::{ProgressBar, ProgressStyle};
        use tokio::io::AsyncWriteExt;
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
        if dest_path.exists() {
            std::fs::remove_file(dest_path).owe_res()?;
        }
        let mut ctx = OperationContext::doing("download url")
            .with_auto_log()
            .with_mod_path("addr/http");
        ctx.record("url", addr.url().as_str());
        let client =
            create_http_client_by_ctrl(self.ctrl().clone().and_then(|x| x.direct_http_ctrl(&addr)))
                .with_context(&ctx)?;
        let mut request = client.get(addr.url());
        if let (Some(u), Some(p)) = (addr.username(), addr.password()) {
            request = request.basic_auth(u, Some(p));
        }

        println!("downlaod from :{}", addr.url());
        let mut response = request.send().await.owe_res().with_context(&ctx)?;

        if !response.status().is_success() {
            return Err(AddrReason::from_res()
                .to_err()
                .doing(format!("HTTP request failed: {}", response.status())))
            .with_context(&ctx);
        }

        let total_size = response.content_length().unwrap_or(0);

        ctx.record("local", dest_path.display().to_string());
        let mut file = tokio::fs::File::create(&dest_path)
            .await
            .owe_conf()
            .with_context(&ctx)?;

        // 创建进度条
        let pb = ProgressBar::new(total_size);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})").owe_logic()?
            .progress_chars("#>-"));

        let mut downloaded: u64 = 0;

        debug!(
            target: "orion_variate::addr::http",
            url = %addr.url(),
            total_size = total_size,
            "starting download stream"
        );
        while let Some(chunk) = response.chunk().await.owe_data().with_context(&ctx)? {
            file.write_all(&chunk).await.owe_sys().with_context(&ctx)?;

            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }

        pb.finish_with_message("下载完成");
        debug!(
            target: "orion_variate::addr::http",
            path = %dest_path.display(),
            "download completed"
        );
        ctx.mark_suc();
        Ok(dest_path.to_path_buf())
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
            return Err(AddrReason::from_res().to_err().doing("path not exist"));
        }
        match addr {
            Address::Http(http) => {
                self.upload(http, path, options.http_method()).await?;
                /*
                if path.is_file() {
                    std::fs::remove_file(path).owe_res()?;
                } else {
                    std::fs::remove_dir_all(path).owe_res()?;
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
    use orion_error::testcase::TestAssertWithMsg;
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
            std::fs::remove_file(&test_file).owe_res()?;
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
            std::fs::remove_file(&test_file).owe_res()?;
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
        let temp_dir = tempfile::tempdir().owe_res()?;
        let file_path = temp_dir.path().join("test.txt");
        tokio::fs::write(&file_path, "test content")
            .await
            .owe_sys()?;

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
        let temp_dir = tempfile::tempdir().owe_res()?;
        let file_path = temp_dir.path().join("test_put.txt");
        tokio::fs::write(&file_path, "test put content")
            .await
            .owe_sys()?;

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
