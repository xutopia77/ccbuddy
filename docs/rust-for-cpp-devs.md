# ccbuddy 后端 Rust 快速上手指南（写给 C++ 开发者）

本指南面向从 C++ 转到本项目的开发者，帮助你把已有的 C++ 心智模型映射到
本项目的 Rust 代码上。C++ 对照统一放在本文件里，代码注释只讲业务逻辑。

---

## 1. 项目架构总览

### 1.1 目录结构

```
src-tauri/
├── Cargo.toml            # ≈ CMakeLists.txt（依赖 + 编译选项）
├── src/
│   ├── lib.rs            # 库根：模块注册 + hook 安装/下载
│   ├── main.rs           # 桌面端入口（调 lib.rs 的 run()）
│   ├── bin/
│   │   ├── ccbuddy-hook.rs     # hook 可执行文件入口（Claude Code 调用）
│   │   └── ccbuddy-server.rs  # 无头服务入口（Linux 服务器）
│   ├── core.rs           # 命令分发（路由表）
│   ├── proto.rs            # RPC 协议结构体 + 错误码
│   ├── server.rs         # HTTP 适配层（axum）
│   ├── claude.rs         # 历史会话管理（读 ~/.claude/projects/）
│   ├── events_mgr.rs     # 事件流管理（读 ~/.ccbuddy/events/）
│   ├── utils.rs          # 共享数据模型与工具函数
│   ├── event.rs          # hook 事件内存模型
│   ├── config.rs         # 用户配置（~/.ccbuddy/config.json）
│   ├── logger.rs         # 日志（等级过滤 + 按大小轮转）
│   └── notify.rs         # 任务栏角标/闪烁（仅 gui feature）
└── tauri.conf.json       # 桌面端打包配置
```

### 1.2 请求流转（调用链）

前端到后端只有一条通道：统一 RPC。

```
前端 (Vue, src/)
   │
   │  invoke("rpc", { payload: { cmd, data } })     [桌面端: Tauri invoke]
   │  fetch("/api/rpc", { body: { cmd, data } })    [服务器端: HTTP POST]
   ▼
┌─────────────────────────────────────────────────────┐
│ lib.rs::gui::rpc()        [桌面端, Tauri command]    │
│ server.rs::rpc_api()      [服务器端, axum handler]   │
└───────────────────────┬─────────────────────────────┘
                        ▼
              core.rs::dispatch(ctx, cmd, data)
                        │  match cmd 路由
        ┌───────────────┼──────────────────┐
        ▼               ▼                  ▼
  events_mgr.rs    claude.rs          config.rs
  (事件流会话)     (历史会话)         (用户配置)
        │               │
        └───────┬───────┘
                ▼
         utils.rs（共享模型：SessionInfo / Message）
                │
                ▼
         proto.rs::ok() → RpcResponse { code, status, data }
                │
                ▼
             前端
```

要点：

- **双数据源**：`events_mgr` 读 hook 实时写的事件流（`~/.ccbuddy/events/`），
  `claude` 读 Claude Code 自己落盘的会话记录（`~/.claude/projects/`）。
  两者解析成同一套展示模型（`utils.rs` 里的 `SessionInfo` / `Message`）。
- **适配层隔离**：Tauri 与 HTTP 的差异被压缩在 `lib.rs::gui` 和 `server.rs`
  两个文件里；业务代码（core / mgr / config 等）是纯 Rust，不碰框架类型。
  新增命令只需改 `core.rs::handle` 的 match 分支 + 前端 `api.ts`。
- **hook 链路**：Claude Code 触发 hook 事件 → 调 `ccbuddy-hook`（独立二进制）
  → stdin 读 JSON → 追加写入 `~/.ccbuddy/events/event-<session-id>.jsonl` →
  `events_mgr` 轮询解析。hook 的原则是"极简、只写入、任何错误不影响主流程"。

### 1.3 构建与测试（对比 CMake）

| C++                          | 本项目（cargo）                                        |
|------------------------------|--------------------------------------------------------|
| `cmake -B build && cmake --build build` | `cargo build`（在 `src-tauri/` 下）          |
| `ctest`                      | `cargo test --lib`                                     |
| 只做语法检查                  | `cargo check`                                          |
| CMake option / profile       | feature：`gui`（桌面端，默认开）、无头构建 `--no-default-features` |
| vcpkg / conan                | `Cargo.toml` 的 `[dependencies]`                       |

---

## 2. 模块导览（职责 + 位置 + C++ 对照）

### lib.rs — crate 根
- **职责**：声明所有子模块（≈ CMake 源文件清单）；hook 的查找、下载、安装、
  注册到 `settings.json`；桌面端 Tauri 入口（`gui` feature 内嵌）。
- **C++ 对照**：`mod xxx;` ≈ `#include` + namespace；`#[cfg(feature)]` ≈
  `#ifdef` 条件编译；tauri Builder 链 ≈ builder 模式。

### core.rs — 命令分发
- **职责**：`dispatch(ctx, cmd, data)` 是唯一的命令路由表，一个命令一个
  match 分支，返回 `Result<Value, AppError>` 统一转 RPC 响应。
- **C++ 对照**：`match cmd` ≈ `switch`；`Result` + `?` ≈ 错误码返回值 +
  早退传播（见下文错误处理节）。

### proto.rs — 协议结构
- **职责**：`RpcRequest` / `RpcResponse` 结构体与状态码常量
  （0 成功 / 400 参数错误 / 500 内部错误）。
- **C++ 对照**：`#[derive(Serialize/Deserialize)]` ≈ protobuf message /
  nlohmann 序列化宏。

### claude.rs — 历史会话（原生 transcript）
- **职责**：扫描 `~/.claude/projects/**/*.jsonl`，概要常驻内存（mtime 缓存，
  启动预热），详情按需全量解析。
- **C++ 对照**：`OnceLock<Mutex<HashMap>>` ≈ Meyers singleton + std::mutex。

### events_mgr.rs — 事件流（hook 日志）
- **职责**：解析 `~/.ccbuddy/events/event-*.jsonl`，用状态机从事件序列推断
  会话状态（运行中/等待确认/等待输入/出错/已完成）；hook 安装状态检测。
- **C++ 对照**：`SessionAgg::apply` ≈ 状态机类的 `void process(const Event&)`；
  `enum SessionStatus` ≈ `enum class`。

### utils.rs — 共享模型
- **职责**：前后端共享的 `SessionInfo` / `Message`（serde 序列化给前端）、
  时间处理、文本截断、系统注入标记识别。
- **C++ 对照**：serde derive ≈ Boost.Serialization / 反射序列化宏。

### event.rs — 事件内存模型
- **职责**：一行 hook 日志 JSON → 内存 `Event` 结构（含 payload 的便捷取值方法）。
- **C++ 对照**：`impl Event` ≈ 类外定义成员函数。

### config.rs — 用户配置
- **职责**：`~/.ccbuddy/config.json` 读写；`ConfigView` 是给前端的 DTO
  （含只读派生字段）。
- **C++ 对照**：`Config`/`ConfigView` 分离 ≈ 持久化对象与 DTO 分开。

### server.rs — HTTP 适配层
- **职责**：axum 路由 `/api/rpc` + 静态资源（前端嵌入二进制）。
- **C++ 对照**：axum ≈ Crow/drogon；`async fn` + `.await` ≈ C++20 协程。

### logger.rs — 日志
- **职责**：`log` crate 门面下的自定义 logger：等级过滤、控制台+文件双写、
  按大小轮转；等级可运行时调整（`set_level`）。
- **C++ 对照**：`impl Log for Logger` ≈ 实现 spdlog 的 sink 接口；
  `Box::leak` ≈ 有意不 delete 的全局单例。

### notify.rs — 任务栏通知（仅 gui）
- **职责**：每 2s 轮询紧急会话，Windows 任务栏角标（overlay icon）+ 闪烁。
- **C++ 对照**：`Mutex<HashSet>` ≈ `std::mutex` + `std::unordered_set`；
  `AtomicBool` ≈ `std::atomic<bool>`；Win32 调用都在显式 `unsafe` 块里。

### bin/ccbuddy-hook.rs — hook 入口
- **职责**：读 stdin → 包装 `{received_at, hook_event, payload}` → 追加写
  按会话分文件的 JSONL。任何错误都退出码 0（不打扰 Claude Code）。

### bin/ccbuddy-server.rs — 无头服务入口
- **职责**：`include_dir!` 嵌入前端 dist，启动 HTTP 服务。

---

## 3. 常用改造场景速查表（C++ ↔ 本项目 Rust）

### 3.1 全局单例 + 互斥锁保护的 map

```cpp
// C++: Meyers singleton + std::mutex
static std::unordered_map<std::string, Session>& cache() {
    static std::unordered_map<std::string, Session> m;
    return m;
}
{
    std::lock_guard<std::mutex> g(mtx);
    cache()[id] = entry;
}
```

```rust
// Rust (claude.rs): OnceLock<Mutex<HashMap>>，get_or_init 保证只建一次
fn summary_cache() -> &'static Mutex<HashMap<String, SummaryEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, SummaryEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

let mut cache = summary_cache().lock().unwrap();  // ≈ lock_guard
cache.insert(id, entry);                          // 出作用域自动解锁
```

### 3.2 智能指针

| C++                    | Rust                    | 说明                                   |
|------------------------|-------------------------|----------------------------------------|
| `std::unique_ptr<T>`   | `Box<T>`                | 独占所有权，move 语义                  |
| `std::shared_ptr<T>`   | `Arc<T>`（跨线程）/ `Rc<T>`（单线程） | 引用计数共享                   |
| 裸指针 / 观察指针       | `&T`（借用）             | Rust 里"借用"由编译器检查生命周期      |
| `new` 后不 delete（全局） | `Box::leak(Box::new(x))` | 有意泄漏成 `&'static`（logger.rs 用到） |

注意：Rust 的"引用"传参不需要写 `shared_ptr`——编译器保证被借用的对象
活过借用期，所以普通函数直接收 `&SessionInfo` 即可，零成本。

### 3.3 回调 / std::function ↔ 闭包

```cpp
std::function<bool(const File&)> pred = [](const File& f) { return f.size > 0; };
auto found = std::find_if(v.begin(), v.end(), pred);
```

```rust
let found = files.iter().find(|p| {
    p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
});
```

闭包写法一致；本项目的约定是**短闭包就地写，长逻辑展开成显式 for 循环 +
命名中间变量**（见下文 3.9）。

### 3.4 枚举

```cpp
enum class Status { Running, Error, Completed };
```

```rust
// events_mgr.rs
enum SessionStatus { Running, WaitingConfirmation, Error, Completed, Idle }
// 相当于给 enum class 加成员函数：
impl SessionStatus {
    fn as_str(&self) -> &'static str { match self { ... } }
}
```

Rust 的 enum 更强：可携带数据（`enum Shape { Circle(f64), Rect(w: f64, h: f64) }`，
≈ tagged union），match 解构时编译器强制穷尽所有分支。

### 3.5 模板 / 泛型

```cpp
template <typename T> T max_of(T a, T b);
```

```rust
fn max_of<T: Ord>(a: T, b: T) -> T   // 泛型，编译期单态化（同模板）
fn take(s: impl Into<String>)        // 按受约束的任意类型（≈ 隐式转换参数）
```

本项目里泛型用得少；`impl Into<String>`（proto.rs 的 AppError 构造）是唯一
常见场景，读作"任何能转成 String 的类型"。

### 3.6 错误处理：Result + ? （本项目核心约定）

C++ 常见做法是异常或错误码；本项目的约定是**错误作为普通值返回**：

```cpp
// C++: 错误码
int load_config(Config* out);  // 0 成功，非 0 失败
```

```rust
// Rust: Result<成功类型, 错误类型>；? 是"失败就早退"的传播运算符
fn save(cfg: &Config) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;  // 失败 → return Err(...)
    Ok(())  // 显式成功
}
```

- `?` 的语义：`expr?` 等价于"expr 失败则把 Err 转换后 return"，比异常
  浅显——**每一处可能的失败在源码上都看得见**。
- 本项目统一用 `Result<T, String>`（业务层）和 `Result<Value, AppError>`
  （RPC 层，AppError 带 code/status），最终在 `core.rs::dispatch` 统一转
  `RpcResponse`，类似在框架层统一 catch。
- `Option<T>` ≈ `std::optional<T>`，`None`/`Some(x)`；`.unwrap()` 是
  "断言非空"（测试代码可用，业务代码慎用）。

### 3.7 线程与锁

```cpp
std::thread t([]{ prewarm(); });
t.detach();
```

```rust
// claude.rs
std::thread::Builder::new()
    .name("claude-mgr-prewarm".into())
    .spawn(prewarm);   // 不 join 即分离运行
```

- `Mutex<T>` 与 C++ 不同的一点：**数据被锁包住**（`Mutex<HashMap>`），
  拿到数据必须先拿锁，编译器强制——不存在"忘了加锁就访问"的写法。
- `lock().unwrap()`：本项目业务代码假定锁不会中毒（持锁线程 panic 即
  进程级 bug），故 unwrap；logger.rs 演示了忽略中毒的写法
  `unwrap_or_else(|p| p.into_inner())`。

### 3.8 RAII / 析构

```cpp
class Guard { public: ~Guard() { cleanup(); } };
```

```rust
// Drop trait ≈ 析构函数，离开作用域自动调用
impl Drop for Guard {
    fn drop(&mut self) { cleanup(); }
}
```

日常场景（MutexGuard 解锁、File 关闭）Rust 全部自动处理，业务代码几乎
不用手写 Drop；std 类型的析构都由标准库实现。

### 3.9 迭代器链 ↔ 显式循环（本项目风格约定）

Rust 惯用长链 `iter().filter().map().collect()`，C++ 开发者读着费劲。
**本项目的约定：1-2 步的短链可保留，更长的展开成命名变量的 for 循环。**

```rust
// 改造前（紧凑但难读）：
load_events()
    .into_iter()
    .filter(|s| matches!(s.status.as_str(), "waiting_confirmation" | "error"))
    .map(|s| s.id)
    .collect()

// 改造后（本项目风格，events_mgr.rs）：
let mut out: Vec<String> = Vec::new();
for session in load_events() {
    let urgent = matches!(
        session.status.as_str(),
        "waiting_confirmation" | "waiting_input" | "error"
    );
    if urgent {
        out.push(session.id);
    }
}
out
```

### 3.10 match（switch 的强化版）

```cpp
switch (type) {
    case "custom-title": ...; break;
    default: break;   // C++ 不强制写 default
}
```

```rust
match t {
    "custom-title" => { ... }
    "summary"      => { ... }
    // 编译器强制穷尽：没有 _ 分支时漏一个 case 编译不过
    _ => {}   // 本项目约定：兜底分支必须写注释说明"吞掉的是什么"
}
```

match 还能解构、加守卫（`"user" if is_sidechain(&v) => {}`），
比 switch 表达力强得多；claude.rs 的 `parse_native_session` 是典型例子。

### 3.11 JSON 处理

```cpp
nlohmann::json v = nlohmann::json::parse(line);
if (v.contains("cwd") && v["cwd"].is_string()) cwd = v["cwd"];
```

```rust
let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
    cwd = c.to_string();
}
```

`serde_json::Value` ≈ `nlohmann::json` 动态树；强类型结构体用
`#[derive(Deserialize)]`（event.rs 的 LogEntry）。

### 3.12 let-else / if-let（失败早退句式）

```rust
// let-else：解析失败就跳过这一行（claude.rs 大量使用）
let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
    continue;   // 或 return / return None
};
// 此后 v 一定有效，不用嵌套 if
```

读法：`let 模式 = 表达式 else { 失败路径 };`——失败路径必须发散
（return/continue/break），所以成功分支不增加缩进。

---

## 4. 本项目的约定

### 4.1 错误处理
- 业务函数返回 `Result<T, String>`，错误消息直接是给用户看的中文说明。
- RPC 层错误是 `AppError { code, status }`（proto.rs），`core.rs::dispatch`
  统一转响应；**不要 panic（unwrap 业务数据）**，用 `ok()` / `?` / let-else。
- 文件解析容忍脏数据：单行解析失败跳过该行，不让一个坏文件毁掉整个列表。

### 4.2 日志
```rust
log::info!("hook 下载完成: {}", dst.display());
log::warn!("命令 {cmd} 失败: code={} status={}", e.code, e.status);
log::debug!("RPC 命令: {cmd}");
```
- 用 `log` crate 的宏（宏 ≈ 编译期展开，`{var}` 直接内插变量名）。
- 落盘位置 `<启动目录>/data/logs/app.log`，等级经 `set_config` 的
  `log_level` 字段运行时可调。

### 4.3 测试
```rust
#[cfg(test)]              // ≈ #ifdef UNIT_TEST，只随 cargo test 编译
mod tests {
    use super::*;
    #[test]               // ≈ TEST(Foo, Bar) / Catch2 TEST_CASE
    fn summary_prefers_custom_title() {
        // 就地写临时文件 → 调函数 → 断言 → 清理
        assert_eq!(info.title, "自定义标题");
    }
}
```
- 测试**内嵌在被测源文件底部**（`#[cfg(test)] mod tests`），不是单独的
  `test/` 目录；好处是测试与实现一起演进，`cargo test --lib` 跑全部。
- 修改后请跑：
  - `cargo test --lib --manifest-path src-tauri/Cargo.toml`（应全过）
  - `cargo check --no-default-features --manifest-path src-tauri/Cargo.toml`
    （无头构建路径不能坏）

### 4.4 命名与风格
- 模块文件小写蛇形（`claude.rs`），类型大驼峰（`SessionAgg`），
  函数小蛇形（`load_events`）——cargo 会拒绝大写开头的文件名。
- 中文注释是本仓库的既定风格，保持；模块头 `//!` 是文档注释，
  相当于文件级说明注释（rustdoc 可渲染成文档）。
- 长迭代链改写为显式 for + 命名中间变量；match 的兜底 `_ =>` 必须注释。

### 4.5 别踩的坑（C++ 开发者常见误区）
1. **不要用可变全局变量模拟状态**：用 `OnceLock<Mutex<...>>` 单例
   （参考 `summary_cache()`），数据放进锁里。
2. **不要忽略 `Result`/`Option`**：编译器会强制你处理——`.ok()` 是显式
   丢弃，写出来就要在注释里说明为什么可以丢。
3. **不要拿 C++ 的 `&` 套 Rust 的引用**：Rust 引用有借用检查（同一时间
   要么多个只读借用，要么一个可变借用），本质是把 C++ 里 dangling
   reference 的运行期错误提前到编译期。
4. **clone 不可耻**：C++ 里拷贝是性能焦虑，本项目大量小字符串/结构体的
   `.clone()`（缓存命中复用时），优先正确与清晰。
5. **没有继承**：不要找基类——共享行为用组合（结构体含字段）或 trait
   （接口）表达，`dyn Trait` 才是虚表指针（本项目几乎没有用到）。

---

## 5. 一页速查

| 你在 C++ 里想的          | 在本项目里写                                            |
|--------------------------|---------------------------------------------------------|
| `std::map` + `std::mutex` | `Mutex<HashMap<String, T>>`（见 claude.rs）      |
| `std::optional`           | `Option<T>`（`Some(x)` / `None`）                       |
| `std::unique_ptr`        | `Box<T>`                                                |
| `std::shared_ptr`        | `Arc<T>`                                                |
| 异常 / 错误码              | `Result<T, String>` + `?`                              |
| `switch`                 | `match`（必须穷尽）                                      |
| `enum class`             | `enum` + `impl`（成员函数写在 impl 块）                 |
| lambda / `std::function` | 闭包（短就地写，长改 for 循环）                          |
| 模板                      | 泛型 `<T: Trait>`；`impl Into<String>` ≈ 隐式转换参数   |
| 析构 / RAII               | `Drop` trait（一般不用手写）                             |
| `static` 局部变量单例      | `OnceLock<T>::new()` + `get_or_init`                    |
| 单元测试目录               | `#[cfg(test)] mod tests` 内嵌源文件底部                 |
| `#ifdef`                  | `#[cfg(feature = "gui")]`                               |
| JSON 库                    | `serde_json::Value`（动态）/ `#[derive(Serialize)]`（强类型） |
