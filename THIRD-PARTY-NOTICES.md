# 第三方组件与许可声明

本应用（小林若葉Manager / endfield-mod-manager）版本 0.1.2。
分发二进制时，请随包附带本文件。

完整依赖清单见仓库中的 `src-tauri/Cargo.lock`（Rust）与 `package-lock.json`（前端）。
**所有组件均为宽松许可或弱著佐权许可，不含 GPL / AGPL 等强著佐权组件。**

## 一、运行时 Rust 组件（静态编译进程序，用户无需另外安装）

### 核心框架

| 组件 | 用途 | 许可 |
|---|---|---|
| `tauri` / `tauri-build` / `tauri-runtime` / `wry` / `tao` | 应用框架与窗口 | MIT OR Apache-2.0 |
| `tauri-plugin-dialog` / `tauri-plugin-opener` | 文件对话框 / 打开外链 | MIT OR Apache-2.0 |
| `serde` / `serde_json` | 序列化 | MIT OR Apache-2.0 |
| `tokio` | 异步运行时 | MIT |
| `trash` | 删除到回收站 | MIT |
| `pinyin` | 角色名拼音排序 | MIT |
| `sha1` / `sha2` | 校验 | MIT OR Apache-2.0 |
| `windows-sys` | Win32 API（单实例检测、权限检测） | MIT OR Apache-2.0 |

### 解压相关

| 组件 | 用途 | 许可 |
|---|---|---|
| `zip` | 解压 `.zip` | MIT |
| `sevenz-rust2` | 解压 `.7z`（纯 Rust，**不包含也不调用 7-Zip 的 `7z.exe`**） | Apache-2.0 OR MIT |
| `unrar` / `unrar_sys` | 解压 `.rar`（静态链接 UnRAR C++ 源码） | MIT OR Apache-2.0（UnRAR 部分见下） |

#### UnRAR 声明（必须保留）

仅使用解压与列表功能，**未实现任何 RAR 压缩功能**。依据 UnRAR 许可要求，完整声明如下：

> UnRAR source code may be used in any software to handle RAR archives
> without limitations free of charge, but cannot be used to develop RAR
> (WinRAR) compatible archiver and to re-create RAR compression algorithm,
> which is proprietary.

Copyright (C) 1993-2024 Alexander Roshal. All copyrights to RAR and the utility
UnRAR are exclusively owned by the author - Alexander Roshal.
UnRAR 以 "AS IS" 形式提供，作者不对数据丢失、损害或利润损失承担任何责任。

### 网络（Mod 市场）

| 组件 | 用途 | 许可 |
|---|---|---|
| `reqwest` | HTTP 客户端 | MIT OR Apache-2.0 |
| `rustls` / `hyper-rustls` / `tokio-rustls` | TLS（不依赖系统 OpenSSL） | Apache-2.0 OR ISC OR MIT |
| `ring` | 加密原语（含 OpenSSL 派生代码） | Apache-2.0 AND ISC |
| `rustls-webpki` / `untrusted` | 证书校验 | ISC |
| **`webpki-roots`** | 内置 CA 根证书数据 | **CDLA-Permissive-2.0** |
| `encoding_rs` | 字符编码转换 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| `brotli` / `brotli-decompressor` | HTTP 压缩解码 | BSD-3-Clause AND MIT |

### 需保留声明的其它许可（均为宽松 / 弱著佐权）

| 许可证 | 相关组件（示例） | 义务说明 |
|---|---|---|
| **MPL-2.0**（文件级弱著佐权） | `cssparser`、`cssparser-macros`、`selectors`、`dtoa-short`、`option-ext` | 本应用**未修改**这些组件源码，按 MPL-2.0 仅需保留其许可声明 |
| **Unicode-3.0** | `icu_*`、`litemap`、`tinystr`、`writeable`、`yoke`、`zerovec` 等 18 个 | Unicode 数据许可，保留声明 |
| **BSD-3-Clause** | `subtle`、`alloc-stdlib`、`alloc-no-stdlib` | 保留版权与免责声明 |
| **ISC** | `ring`、`rustls-webpki`、`untrusted` | 保留版权声明 |
| **Unlicense / MIT** | `memchr`、`aho-corasick`、`walkdir`、`byteorder`、`jiff` 等 | 双许可，按 MIT 使用 |
| **Zlib** | `foldhash`、`zlib-rs`、`miniz_oxide` | 保留来源声明 |
| **BSL-1.0**（Boost） | `ryu` | 保留版权声明 |
| **0BSD / CC0-1.0** | `adler2`、`dunce` | 无署名义务 |

> 上表为许可证分布中的代表项；其余组件为 MIT 或 Apache-2.0。全部依赖包的确切许可见 `Cargo.lock`。

## 二、前端组件（打包进 `dist/`）

| 组件 | 版本 | 许可 |
|---|---|---|
| `react` / `react-dom` | 19.x | MIT |
| `@tauri-apps/api` | 2.x | Apache-2.0 OR MIT |
| `@tauri-apps/plugin-dialog` | 2.x | MIT OR Apache-2.0 |
| `@tauri-apps/plugin-opener` | 2.x | MIT OR Apache-2.0 |

构建工具（Vite、TypeScript、@vitejs/plugin-react、@types/*）仅在开发与打包阶段使用，
其代码**不会进入发行产物**。

## 三、数据与素材

- **GameBanana**：Mod 列表、详情、评论、分类与下载链接来自其公开 API
  （https://gamebanana.com）。使用时应遵守 GameBanana 服务条款与 API 规范，
  请勿高频抓取或盗链批量分发图片。
- 各游戏的角色立绘、游戏图标等素材版权归各自游戏发行商所有，本应用仅作识别与展示用途。

## 四、设计参考

功能设计与交互参考了以下项目，**未包含其源代码**：

- **IMM — Integrated Mod Manager**（https://github.com/jpbhatt21/integrated-mod-manager）
  该仓库未附带开源许可证，因此本项目未复制其代码，仅在功能行为与界面组织上作参考。
- **QAQM**：同类模组管理器，仅作功能与交互参考；本应用继承了其 Mod 目录结构以便用户迁移配置。
