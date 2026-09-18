## [0.8.2] - 2026-09-18

### Changed

- 升级 `git2` 至 0.21（规避 RUSTSEC-2026-0183 / RUSTSEC-2026-0184），适配 `shorthand()` / `name()` 返回 `Result` 的 API 变更。
- 升级 `reqwest` 至 0.13。
- 仓库地址由 `galaxy-sec` 更正为 `galaxio-labs`：README 徽章、`Cargo.toml` 的 `repository`、测试夹具 URL（旧地址返回 `Repository not found`）。

### Added

- 新增 `LICENSE`（MIT）。此前 `Cargo.toml` 已声明 `license = "MIT"`，但文件和 README 链接都缺失。

### Fixed

- 补齐 `git2` 的 `ssh` / `https` feature。0.21 起 `default = []`，仅声明 `vendored-openssl` 会让 libgit2 缺失 TLS 后端，HTTPS 克隆报 `there is no TLS stream available`。
- CI 的 Security Audit job 补上 `cargo generate-lockfile`。`Cargo.lock` 不入库，而 `rustsec/audit-check` 写死了 `--file ./Cargo.lock`。

### Dependencies

- `git2`: `0.20` → `0.21`
- `reqwest`: `0.12` → `0.13`
- `rstest` (dev): `0.26` → `0.27`

## [0.8.0] - 2026-05-03

### Changed

- 升级 `orion-error` 至 0.8，适配 breaking API 变更。
- `AddrReason` 改为 `#[derive(OrionError)]`，替代手写 `DomainReason` / `ErrorCode` impl。
  - `Brief` variant：`identity = "biz.addr.brief"`，`code = 500`。
  - `Unified` variant：`#[orion_error(transparent)]` 委托给 `UnifiedReason`。
  - 带字段 variant（`OperationTimeoutExceeded`、`TotalTimeoutExceeded`、`RetryExhausted`）各自有 `identity` + `code` + `message`。
- 移除 `compat_traits::ErrorOwe` / `traits_ext`：`.owe_*()` → `.map_err(raw_err).source_err(...)` 或 `.source_err(...)`。
- `UvsReason` → `UnifiedReason`，`UvsFrom` / `from_res()` / `from_biz()` / `from_conf()` / `from_data()` → `AddrReason::*()` 构造函数。
- `DomainReason` / `ErrorCode` 导入路径从 crate root 移至 `orion_error::reason`。
- `testcase` → `dev::testing`。

### Added

- `src/raw.rs`：`RawErr<E>` + `raw_err()`，利用 `RawStdError` 绕过孤儿规则，使任意第三方 `StdError`（`git2::Error`、`reqwest::Error`）可进入 `.source_err()` 路径。
- `AddrReason::*()` 委托构造函数（`system_error()`、`resource_error()`、`data_error()` 等 15 个），由 `#[derive(OrionError)]` 在 `transparent` variant 上自动生成。

### Dependencies

- `orion-error`: `0.7` → `0.8`

## [0.7.0] - 2026-04-25

### Changed
- 升级 `orion-error` 至 0.7。
- `.want()` → `.doing()`，`.with()` → `.with_context()`，适配 orion-error 0.7 API。
- `addr::accessor::create_http_client_by_ctrl` now returns `AddrResult<reqwest::Client>`.
- Refined `GitRepository` builder API: consolidated optional setters and de-duplicated credential loading.
