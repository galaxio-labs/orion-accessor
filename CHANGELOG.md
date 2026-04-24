## [0.7.0] - 2026-04-25

### Changed
- 升级 `orion-error` 至 0.7。
- `.want()` → `.doing()`，`.with()` → `.with_context()`，适配 orion-error 0.7 API。
- `addr::accessor::create_http_client_by_ctrl` now returns `AddrResult<reqwest::Client>`.
- Refined `GitRepository` builder API: consolidated optional setters and de-duplicated credential loading.
