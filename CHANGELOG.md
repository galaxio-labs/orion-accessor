## [0.8.2] - 2026-09-18

### Changed

- 升级 `git2` 至 0.21，规避 `git2` 0.20 的 RUSTSEC-2026-0183 / RUSTSEC-2026-0184（0.21.0 起已修复）。
  - 适配其 API 变更：`Repository::head()` 的 `shorthand()` 与 `Reference::name()` 由 `Option<&str>` 改为 `Result<&str, git2::Error>`；`src/addr/accessor/git.rs` 的 4 处调用相应调整，取不到名称时依旧沿用原分支行为。
- 升级 `reqwest` 至 0.13。
- README 徽章的仓库地址由历史的 `galaxy-sec` 更正为 `galaxio-labs`；CI 徽章改用非弃用的 `actions/workflows/<file>` 形式并固定 `?branch=main`。
- `Cargo.toml` 的 `repository` 字段同步更正为 `galaxio-labs`（已发布版本的元数据不会变，需下次发布后在 crates.io 生效）。

### Added

- 新增 `LICENSE`（MIT，`Copyright (c) 2025 galaxio-labs`）。此前仓库缺少许可文件，但 `Cargo.toml` 已声明 `license = "MIT"`，README 末尾的 `[LICENSE](LICENSE)` 也是死链。

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
