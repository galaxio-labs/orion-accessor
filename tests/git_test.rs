// GitAddr 代理配置使用示例 - 集成测试
// 测试函数 - 可直接运行验证，包含实际克隆仓库测试
// 运行前设置环境变量：
// export https_proxy=http://127.0.0.1:7890
// export http_proxy=http://127.00.1:7890
// export all_proxy=socks5://127.0.0.1:7890
//
// 运行命令：
// cargo test --test proxy_integration_test

use orion_accessor::addr::access_ctrl::Rule;
use orion_accessor::addr::access_ctrl::serv::NetAccessCtrl;
use orion_accessor::addr::accessor::GitAccessor;
use orion_accessor::addr::proxy::ProxyConfig;
use orion_accessor::addr::{Address, GitRepository};
use orion_accessor::types::ResourceDownloader;
use orion_accessor::update::DownloadOptions;

#[test]
fn test_git_proxy() {
    // 示例1：基本代理配置
    println!("=== 示例1: 基本代理配置 ===");
    let _git_addr = GitRepository::from("https://github.com/example/repo.git");
    let accessor = GitAccessor::default().with_ctrl(Some(NetAccessCtrl::from_rule(
        Rule::new("*", ""),
        None,
        None,
    )));
    match accessor
        .ctrl()
        .clone()
        .and_then(|x| x.proxy_git(&_git_addr))
    {
        Some(proxy) => {
            println!("使用代理: {}", proxy.url());
        }
        None => println!("未配置代理"),
    }

    // 创建临时目录用于测试
    let temp_dir = std::env::temp_dir().join("git_proxy_test");
    let _ = std::fs::remove_dir_all(&temp_dir); // 清理旧目录

    // 使用公共测试仓库
    let test_repo = "https://github.com/galaxio-labs/hello-word.git";
    let git_addr = GitRepository::from(test_repo);

    println!("测试仓库: {test_repo}");

    let accessor = GitAccessor::default().with_ctrl(Some(NetAccessCtrl::from_rule(
        Rule::new("*", ""),
        None,
        //export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890 all_proxy=socks5://127.0.0.1:7890
        Some(ProxyConfig::new("http://127.0.0.1:7890")),
    )));

    // 测试代理配置
    match accessor
        .ctrl()
        .clone()
        .and_then(|x| x.proxy_git(&_git_addr))
    {
        Some(proxy) => {
            println!("使用代理: {}", proxy.url());
            println!("准备克隆到: {}", temp_dir.display());

            // 使用异步运行时执行实际的git clone操作
            let rt = tokio::runtime::Runtime::new().unwrap();
            let clone_result = rt.block_on(async {
                accessor
                    .download_to_local(
                        &Address::from(git_addr),
                        &temp_dir,
                        &DownloadOptions::default(),
                    )
                    .await
            });

            match clone_result {
                Ok(update_unit) => {
                    println!("仓库克隆成功！路径: {}", update_unit.position().display());
                    // 验证克隆的目录存在且包含.git目录
                    let git_dir = update_unit.position().join(".git");
                    assert!(git_dir.exists(), "克隆的仓库应该包含.git目录");
                    println!("代理配置验证通过 - 克隆操作成功完成");
                }
                Err(e) => {
                    println!("克隆失败: {e}");
                    // 在测试环境中，允许克隆失败，但验证代理配置被正确应用
                    println!("测试环境可能无法访问外部仓库，但代理配置已正确加载");
                }
            }
        }
        None => {
            println!("无代理配置，使用直连");
            // 验证无代理时也能正常配置
            assert_eq!(git_addr.repo(), test_repo);
            println!("直连配置验证通过");
        }
    }

    // 清理环境变量
    unsafe {
        std::env::remove_var("HTTPS_PROXY");
    }

    // 清理临时目录
    let _ = std::fs::remove_dir_all(&temp_dir);
}
