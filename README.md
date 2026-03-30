# WebdavBridge

WebDAV 代理桥接服务，提供元数据缓存、速率限制和符号链接功能，适用于视频/音频流媒体场景。

## 目录

- [项目概述](#项目概述)
- [核心特性](#核心特性)
- [系统架构](#系统架构)
- [符号链接功能](#符号链接功能)
- [安装与构建](#安装与构建)
- [配置说明](#配置说明)
- [使用指南](#使用指南)
- [API 参考](#api-参考)
- [项目结构](#项目结构)
- [开发指南](#开发指南)
- [许可证](#许可证)

---

## 项目概述

WebdavBridge 作为下游 WebDAV 客户端和上游 WebDAV 服务器之间的代理桥接层。它不仅提供缓存和限流功能以减轻上游服务器压力，还通过创新的符号链接机制实现了上下游完全解耦——下游的所有写操作（COPY/MOVE/PUT/DELETE）都不会影响上游服务器。

### 适用场景

- **流媒体服务**：为视频/音频文件提供缓存代理，支持断点续传
- **只读镜像+本地修改**：在不修改上游的前提下，下游可自由组织文件结构
- **多客户端共享**：多个下游客户端共享同一上游资源，各自独立管理符号链接
- **带宽节省**：通过本地缓存和元数据缓存减少对上游的重复请求

---

## 核心特性

| 特性 | 说明 |
|------|------|
| **元数据缓存** | 使用 sled 嵌入式数据库缓存目录结构和文件元数据，加速 PROPFIND 响应 |
| **内容缓存** | 基于本地文件系统缓存已请求的文件内容，避免重复下载 |
| **速率限制** | 基于信号量的并发控制，防止过多请求压垮上游服务器 |
| **Range 请求** | 完整支持单范围和多范围（multipart/byteranges）请求，适配流媒体播放器 |
| **符号链接** | 下游 COPY/MOVE 创建符号链接，PUT 写入本地覆盖，DELETE 仅删除下游数据 |
| **循环检测** | 自动检测符号链接循环引用，防止无限递归 |
| **后台同步** | 定期从上游同步元数据，保持目录结构更新 |
| **断点续传** | 支持 ETag、If-Range、Content-Range 等 HTTP 头部 |
| **向后兼容** | 符号链接字段使用 serde 默认值，旧版缓存数据可无缝升级 |

---

## 系统架构

```
┌──────────────────────────────────────────────────┐
│           下游 WebDAV 客户端                       │
│   GET / HEAD / PROPFIND / COPY / MOVE / PUT / DELETE │
└─────────────────────┬────────────────────────────┘
                      │
                      ▼
         ┌────────────────────────┐
         │   WebdavBridge 服务    │
         │   (hyper HTTP 服务器)  │
         └────────┬───────────────┘
                  │
        ┌─────────┼─────────────┐
        │         │             │
        ▼         ▼             ▼
  ┌──────────┐ ┌──────────┐ ┌────────────────┐
  │ Metadata │ │ Content  │ │ ContentFetch   │
  │ Cache    │ │ Cache    │ │ Task           │
  │ (sled)   │ │ (文件系统)│ │ (单线程通道)   │
  └──────────┘ └──────────┘ └───────┬────────┘
        │                           │
        │                    ┌──────▼──────────┐
        │                    │ UpstreamClient  │
        │                    │ (速率限制)       │
        │                    └──────┬──────────┘
        │                           │
  ┌─────▼───────────┐              │
  │ MetadataUpdate  │              │
  │ Task (定期同步)  │              │
  └────────┬────────┘              │
           │                       │
           └───────────┬───────────┘
                       │
                       ▼
            ┌────────────────────┐
            │  上游 WebDAV 服务器 │
            │  （第三方/远程）     │
            └────────────────────┘
```

### 数据流说明

| 请求类型 | 数据流 |
|---------|--------|
| **GET** | 客户端 → WebdavServer → ContentFetchTask → 本地缓存 / 上游 → 返回内容 |
| **HEAD** | 客户端 → WebdavServer → ContentFetchTask → 元数据缓存 / 上游 HEAD → 返回元数据 |
| **PROPFIND** | 客户端 → WebdavServer → MetadataCache → 返回目录列表 |
| **COPY** | 客户端 → WebdavServer → MetadataCache（创建符号链接）→ 返回 201/204 |
| **MOVE** | 客户端 → WebdavServer → MetadataCache（移动符号链接）→ 返回 201/204 |
| **PUT** | 客户端 → WebdavServer → ContentCache（写入本地）→ 返回 204 |
| **DELETE** | 客户端 → WebdavServer → MetadataCache + ContentCache（删除）→ 返回 204 |

### 核心组件

| 组件 | 文件 | 说明 |
|------|------|------|
| **WebdavServer** | `src/webdav/server.rs` | 下游请求处理器，处理所有 WebDAV 方法 |
| **UpstreamClient** | `src/webdav/client.rs` | 上游 WebDAV 客户端，支持 PROPFIND/HEAD/GET |
| **DavFs** | `src/webdav/dav_fs.rs` | dav-server 的 DavFileSystem 接口实现 |
| **MetadataCache** | `src/cache/metadata.rs` | 基于 sled 的元数据持久化存储 |
| **ContentCache** | `src/cache/content.rs` | 基于文件系统的内容缓存 |
| **ContentFetchTask** | `src/tasks/content_fetch.rs` | 单线程内容获取任务（通道模式） |
| **MetadataUpdateTask** | `src/tasks/metadata_update.rs` | 后台元数据同步任务 |
| **RateLimiter** | `src/resume/mod.rs` | 基于信号量的并发限制器 |

---

## 符号链接功能

### 设计理念

WebdavBridge 的符号链接功能实现了**上下游完全解耦**：

- 下游客户端可以自由组织文件结构（通过 COPY/MOVE）
- 写入操作只影响本地，上游服务器完全无感知
- 读取操作可透明地映射到上游文件内容
- 支持目录级别的递归符号链接

### 操作详解

#### COPY（创建符号链接）

```
请求：COPY /upstream/video.mp4
头部：Destination: http://host/local/my-video.mp4

处理流程：
1. 查找源路径 /upstream/video.mp4 的元数据
2. 如果源本身是符号链接，使用其 symlink_target 作为新链接的目标
3. 如果源是普通文件，以源路径作为新链接的目标
4. 进行循环引用检测
5. 在 MetadataCache 中创建新条目：
   - path: /local/my-video.mp4
   - is_symlink: true
   - symlink_target: /upstream/video.mp4
6. 返回 201 Created（新建）或 204 No Content（覆盖）

注意：整个过程不访问上游服务器，不复制实际内容
```

#### COPY 目录（递归符号链接）

```
请求：COPY /upstream/movies/
头部：Destination: http://host/local/movies/

处理流程：
1. 创建目录符号链接 /local/movies/ → /upstream/movies/
2. 遍历 /upstream/movies/ 的所有子项
3. 递归为每个子文件/子目录创建对应的符号链接
   - /local/movies/a.mp4 → /upstream/movies/a.mp4
   - /local/movies/b.mp4 → /upstream/movies/b.mp4
   - /local/movies/subdir/ → /upstream/movies/subdir/ (递归)
```

#### MOVE（移动符号链接）

```
请求：MOVE /local/my-video.mp4
头部：Destination: http://host/local/renamed.mp4

处理流程：
1. 验证源路径是符号链接（非符号链接不允许 MOVE，返回 403）
2. 更新路径和名称，symlink_target 保持不变
3. 删除旧路径条目，创建新路径条目
4. 返回 201 Created（新建）或 204 No Content（覆盖）

关键：移动的只是下游的链接路径，指向上游的目标不变
```

#### GET（读取符号链接）

```
请求：GET /local/my-video.mp4

处理流程：
1. 在 MetadataCache 中查找 /local/my-video.mp4
2. 如果是符号链接 且 has_local_override=true：
   → 从 ContentCache（本地文件系统）读取内容
3. 如果是符号链接 且 has_local_override=false：
   → 通过 ContentFetchTask 从上游获取（经过 RateLimiter 限流）
   → 如果上游返回 404，自动删除此符号链接并返回 404
4. 如果不是符号链接：
   → 使用原有的获取逻辑（缓存 + 上游）

重要：读取符号链接的上游请求必须经过 RateLimiter，保持并发限制
```

#### PUT（写入本地覆盖）

```
请求：PUT /local/my-video.mp4
请求体：<文件内容>

处理流程：
1. 验证目标是符号链接（非符号链接不允许 PUT，返回 403）
2. 将内容写入 ContentCache 本地文件系统
3. 在 MetadataCache 中设置 has_local_override=true
4. 后续 GET 请求将返回本地内容而非上游内容
5. 返回 204 No Content

关键：上游服务器完全不受影响
```

#### DELETE（删除符号链接）

```
请求：DELETE /local/my-video.mp4

处理流程：
1. 验证目标是符号链接（非符号链接不允许 DELETE，返回 403）
2. 如果有本地覆盖（has_local_override=true），从 ContentCache 删除
3. 如果是目录，递归删除所有子项
4. 从 MetadataCache 删除条目
5. 返回 204 No Content

关键：上游文件不受任何影响
```

### 循环引用检测

创建符号链接时自动进行循环检测，防止以下场景：

```
直接循环：  A → B → A          ❌ 被阻止
间接循环：  A → B → C → A      ❌ 被阻止
深度超限：  A → B → C → D → E  ❌ 超过 max_symlink_depth 被阻止
正常链接：  A → /upstream/file  ✅ 允许
```

检测算法使用深度优先搜索，从目标路径开始沿符号链接链追踪：
- 如果回到了源路径 → 报告 **SymlinkCycle** 错误
- 如果链长度超过 `max_symlink_depth` → 报告 **SymlinkDepthExceeded** 错误
- 如果链终止于非符号链接或不存在的路径 → 安全，允许创建

### WebdavResource 数据结构

```rust
pub struct WebdavResource {
    pub path: String,                    // 资源路径
    pub name: String,                    // 显示名称
    pub content_type: Option<String>,    // MIME 类型
    pub size: u64,                       // 文件大小（字节）
    pub etag: Option<String>,            // 缓存验证标签
    pub modified: Option<DateTime<Utc>>, // 最后修改时间
    pub is_dir: bool,                    // 是否为目录
    pub supports_resume: bool,           // 是否支持断点续传
    pub is_symlink: bool,                // 是否为符号链接 (新增)
    pub symlink_target: Option<String>,  // 符号链接目标路径 (新增)
    pub has_local_override: bool,        // 是否有本地覆盖 (新增)
}
```

### 安全限制

| 限制 | 说明 |
|------|------|
| 仅符号链接可写 | 非符号链接的 MOVE/PUT/DELETE 返回 403 Forbidden |
| 循环检测 | 自动检测并阻止循环引用的符号链接 |
| 深度限制 | 符号链接链最大深度由 `max_symlink_depth` 控制（默认 3） |
| 上游只读 | 所有写操作仅在下游生效，上游服务器永远不会被修改 |
| 限流保护 | 符号链接读取上游内容时必须经过 RateLimiter |

---

## 安装与构建

### 前置要求

- Rust 工具链（推荐 1.70+）
- Cargo 包管理器

### 构建

```bash
# 克隆仓库
git clone https://github.com/ModerRAS/WebdavBridge.git
cd WebdavBridge

# 开发模式构建
cargo build

# 发布模式构建（优化）
cargo build --release
```

### 运行测试

```bash
cargo test
```

---

## 配置说明

### 配置文件

WebdavBridge 从 YAML 配置文件加载配置，默认路径为 `config.yaml`，可通过环境变量 `WEBDAV_BRIDGE_CONFIG` 指定：

```yaml
# 上游 WebDAV 服务器地址
upstream_url: "http://upstream-server:8081"

# 上游认证（可选）
upstream_username: null
upstream_password: null

# 本地内容缓存目录
cache_dir: "./cache"

# sled 元数据数据库路径
metadata_db_path: "./metadata.db"

# 上游最大并发连接数
rate_limit_permits: 2

# 元数据后台同步间隔（秒）
metadata_update_interval_secs: 300

# PROPFIND 最大递归深度
max_depth: 10

# 服务监听地址
server_bind: "127.0.0.1:8080"

# URL 前缀路径
server_prefix: "/"

# 符号链接最大嵌套深度
max_symlink_depth: 3
```

### 配置项详解

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `upstream_url` | 字符串 | `http://localhost:8081` | 上游 WebDAV 服务器的完整 URL |
| `upstream_username` | 字符串/null | `null` | HTTP Basic 认证用户名 |
| `upstream_password` | 字符串/null | `null` | HTTP Basic 认证密码 |
| `cache_dir` | 路径 | `./cache` | 本地文件内容缓存目录，支持相对和绝对路径 |
| `metadata_db_path` | 路径 | `./metadata.db` | sled 嵌入式数据库存储路径 |
| `rate_limit_permits` | 整数 | `2` | 对上游服务器的最大并发请求数 |
| `metadata_update_interval_secs` | 整数 | `300` | 后台自动同步上游元数据的间隔（秒） |
| `max_depth` | 整数 | `10` | PROPFIND 递归遍历的最大深度 |
| `server_bind` | 字符串 | `127.0.0.1:8080` | 下游 HTTP 服务器监听地址和端口 |
| `server_prefix` | 字符串 | `/` | WebDAV 服务的 URL 前缀路径 |
| `max_symlink_depth` | 整数 | `3` | 符号链接链的最大解析深度，防止循环和过深链 |

---

## 使用指南

### 启动服务

```bash
# 使用默认配置文件 (config.yaml)
cargo run

# 指定配置文件
WEBDAV_BRIDGE_CONFIG=/path/to/config.yaml cargo run

# 设置日志级别
RUST_LOG=webdav_bridge=debug cargo run
```

### 常用操作示例

#### 浏览目录

```bash
# 列出根目录
curl -X PROPFIND http://localhost:8080/

# 列出子目录
curl -X PROPFIND http://localhost:8080/movies/
```

#### 获取文件

```bash
# 下载完整文件
curl http://localhost:8080/movies/video.mp4 -o video.mp4

# 获取文件头部信息
curl -I http://localhost:8080/movies/video.mp4

# 范围请求（适合流媒体）
curl -H "Range: bytes=0-1023" http://localhost:8080/movies/video.mp4

# 带条件的范围请求
curl -H "Range: bytes=1024-2047" \
     -H 'If-Range: "etag-value"' \
     http://localhost:8080/movies/video.mp4
```

#### 符号链接操作

```bash
# 创建符号链接（COPY）
curl -X COPY \
  -H "Destination: http://localhost:8080/local/my-video.mp4" \
  http://localhost:8080/movies/video.mp4
# 返回：201 Created

# 创建目录符号链接
curl -X COPY \
  -H "Destination: http://localhost:8080/local/my-movies/" \
  http://localhost:8080/movies/
# 返回：201 Created（递归创建所有子项的符号链接）

# COPY 不覆盖已存在的目标
curl -X COPY \
  -H "Destination: http://localhost:8080/local/my-video.mp4" \
  -H "Overwrite: F" \
  http://localhost:8080/movies/video.mp4
# 如果目标已存在，返回：412 Precondition Failed

# 移动符号链接（MOVE）
curl -X MOVE \
  -H "Destination: http://localhost:8080/local/renamed.mp4" \
  http://localhost:8080/local/my-video.mp4
# 返回：201 Created

# 写入本地覆盖（PUT）
curl -X PUT \
  --data-binary @new-content.mp4 \
  http://localhost:8080/local/renamed.mp4
# 返回：204 No Content

# 读取（自动返回本地覆盖版本）
curl http://localhost:8080/local/renamed.mp4 -o output.mp4

# 删除符号链接（DELETE）
curl -X DELETE http://localhost:8080/local/renamed.mp4
# 返回：204 No Content
```

---

## API 参考

### 支持的 WebDAV 方法

| 方法 | 路径 | 说明 | 状态码 |
|------|------|------|--------|
| `GET` | 任意文件路径 | 获取文件内容，支持 Range 请求 | 200, 206, 404 |
| `HEAD` | 任意文件路径 | 获取文件元数据 | 200, 404 |
| `PROPFIND` | 任意目录路径 | 列出目录内容 | 207, 404 |
| `COPY` | 源路径 + Destination 头 | 创建符号链接 | 201, 204, 400, 404, 412 |
| `MOVE` | 源路径 + Destination 头 | 移动符号链接 | 201, 204, 403, 404, 412 |
| `PUT` | 符号链接路径 + 请求体 | 写入本地覆盖 | 204, 403, 404 |
| `DELETE` | 符号链接路径 | 删除符号链接 | 204, 403, 404 |

### HTTP 头部支持

| 头部 | 方法 | 说明 |
|------|------|------|
| `Range` | GET | 范围请求，如 `bytes=0-1023` |
| `If-Range` | GET | 条件范围请求（ETag 或日期） |
| `Content-Range` | GET 响应 | 范围响应的字节范围信息 |
| `ETag` | GET/HEAD 响应 | 资源版本标识 |
| `Accept-Ranges` | HEAD 响应 | 指示支持范围请求（`bytes`） |
| `Destination` | COPY/MOVE | 目标路径（完整 URL 或相对路径） |
| `Overwrite` | COPY/MOVE | 是否覆盖已存在的目标（`T`/`F`，默认 `T`） |
| `Depth` | PROPFIND 响应 | 目录深度 |

### 错误响应

| 状态码 | 说明 |
|--------|------|
| 200 | 成功（GET 完整文件） |
| 201 | 已创建（COPY/MOVE 新目标） |
| 204 | 无内容（COPY/MOVE 覆盖，PUT 成功，DELETE 成功） |
| 206 | 部分内容（Range 请求） |
| 207 | 多状态（PROPFIND 目录列表） |
| 400 | 错误请求（缺少 Destination 头、循环引用、深度超限） |
| 403 | 禁止（对非符号链接执行 MOVE/PUT/DELETE） |
| 404 | 未找到（资源不存在或上游目标已删除） |
| 405 | 方法不允许（不支持的 HTTP 方法） |
| 412 | 前提条件失败（Overwrite=F 且目标已存在） |
| 416 | 范围不满足（Range 请求超出文件大小） |
| 500 | 内部错误 |

---

## 项目结构

```
WebdavBridge/
├── Cargo.toml                    # 项目依赖和元数据
├── Cargo.lock                    # 依赖锁定文件
├── README.md                     # 项目文档（本文件）
├── LICENSE                       # MIT 许可证
└── src/
    ├── main.rs                   # 二进制入口，HTTP 路由和服务器启动
    ├── lib.rs                    # 库根模块
    ├── config.rs                 # 配置管理（YAML 读写、默认值）
    ├── webdav/
    │   ├── mod.rs                # WebDAV 模块导出
    │   ├── types.rs              # 核心类型定义
    │   │                         #   - WebdavResource（含符号链接字段）
    │   │                         #   - RangeSpec / MultiRangeSpec
    │   │                         #   - WebdavError（含符号链接错误类型）
    │   │                         #   - CacheStatus
    │   ├── server.rs             # 下游 WebDAV 请求处理器
    │   │                         #   - handle_get()（符号链接感知）
    │   │                         #   - handle_head()
    │   │                         #   - handle_propfind()
    │   │                         #   - handle_copy()（创建符号链接）
    │   │                         #   - handle_move()（移动符号链接）
    │   │                         #   - handle_put()（本地覆盖写入）
    │   │                         #   - handle_delete()（删除符号链接）
    │   ├── client.rs             # 上游 WebDAV 客户端
    │   │                         #   - propfind()（目录列表）
    │   │                         #   - head()（元数据获取）
    │   │                         #   - get_range()（范围内容获取）
    │   └── dav_fs.rs             # dav-server DavFileSystem 接口实现
    │                             #   - open() / read_dir() / metadata()
    │                             #   - copy() / rename()（符号链接操作）
    │                             #   - remove_file() / remove_dir()
    ├── cache/
    │   ├── mod.rs                # 缓存模块导出
    │   ├── metadata.rs           # 元数据缓存（sled 嵌入式数据库）
    │   │                         #   - get() / put() / delete()
    │   │                         #   - get_children() / iter_all()
    │   │                         #   - is_symlink() / get_symlink_target()
    │   │                         #   - set_local_override() / has_local_override()
    │   │                         #   - get_by_target() / delete_by_target()
    │   │                         #   - check_symlink_safety()（循环检测）
    │   └── content.rs            # 内容缓存（文件系统）
    │                             #   - read_range() / write_stream()
    │                             #   - exists() / delete()
    ├── tasks/
    │   ├── mod.rs                # 任务模块导出
    │   ├── metadata_update.rs    # 后台元数据同步任务
    │   │                         #   - 定期 PROPFIND 遍历上游目录
    │   │                         #   - 更新 MetadataCache
    │   └── content_fetch.rs      # 内容获取任务（单线程通道模式）
    │                             #   - fetch()（获取内容）
    │                             #   - get_metadata()（获取元数据）
    └── resume/
        ├── mod.rs                # RateLimiter 速率限制器
        └── range.rs              # Range 头部解析与格式化
                                  #   - parse_range_header()
                                  #   - format_multipart_ranges()
                                  #   - format_content_range()
```

### 依赖说明

| 类别 | 依赖 | 说明 |
|------|------|------|
| HTTP 服务器 | hyper 1, http 1, hyper-util | 异步 HTTP/1.1 服务器 |
| HTTP 客户端 | reqwest 0.12 | 带 TLS 的异步 HTTP 客户端 |
| WebDAV | dav-server 0.5 | WebDAV 协议实现框架 |
| 异步运行时 | tokio 1 | 异步任务调度 |
| 数据库 | sled 0.34 | 嵌入式键值存储 |
| 序列化 | serde, serde_json, serde_yaml | 数据序列化/反序列化 |
| 时间 | chrono 0.4 | 日期时间处理 |
| 流处理 | bytes 1, futures-util 0.3 | 字节流处理 |
| 日志 | tracing, tracing-subscriber | 结构化日志 |
| 错误处理 | thiserror, anyhow | 错误类型定义 |
| 测试 | tokio-test, tempfile, mockito | 异步测试、临时文件、HTTP Mock |

---

## 开发指南

### 构建与测试

```bash
# 构建
cargo build

# 运行所有测试
cargo test

# 运行特定模块测试
cargo test cache::metadata::tests
cargo test webdav::server::tests
cargo test webdav::types::tests

# 带日志运行测试
RUST_LOG=debug cargo test -- --nocapture
```

### 测试覆盖

项目包含 94 个单元测试，覆盖以下方面：

| 模块 | 测试数量 | 覆盖内容 |
|------|---------|---------|
| `webdav::types` | 20 | Range 解析、WebdavResource 构造器、符号链接字段序列化、向后兼容 |
| `webdav::server` | 18 | 响应字段、COPY/MOVE/PUT/DELETE 处理器、目录递归、循环检测、错误场景 |
| `webdav::client` | 2 | 客户端创建、URL 构建 |
| `cache::metadata` | 14 | CRUD 操作、符号链接查询、目标反查、级联删除、循环检测算法 |
| `cache::content` | 3 | 读写、范围读取、删除 |
| `resume::range` | 26 | 范围解析、多范围、内容范围格式化、边界情况 |
| `resume` | 4 | 速率限制器、并发控制 |
| `config` | 2 | 默认配置、保存/加载 |
| `tasks` | 2 | 任务创建 |

### 添加新功能

1. 在 `src/webdav/types.rs` 中定义新的数据类型或错误变体
2. 在 `src/webdav/server.rs` 中实现新的处理器方法
3. 在 `src/main.rs` 中添加 HTTP 路由
4. 在 `src/webdav/dav_fs.rs` 中实现 DavFileSystem 接口方法（如需要）
5. 编写单元测试

---

## 许可证

MIT