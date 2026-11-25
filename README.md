# 小林若葉Manager（KobayashiWakaba Mod Manager）

面向多游戏 XXMI 生态的 Mod 管理器，**QAQM 的 Rust 复刻版**。

本工具以 **QAQM**（质量与可用性俱佳的 XXMI 系 Mod 管理器）为蓝本，使用 **Rust + Tauri 2** 重新实现，保留了 QAQM 的 Mod 目录结构约定，可直接接管其既有配置，并针对 `明日方舟：终末地`、`崩坏：星穹铁道`、`鸣潮`、`绝区零`、`原神` 等游戏做了原生支持。

---

## ✨ 主要功能

- **角色一览**：以角色为维度管理 Mod 作用对象，支持卡片拖动排序（顶部选择「自定义」排序）。
- **Mod 市场**：调用 GameBanana 官方 API，支持一键下载安装、更新检测与「仅下载」。
- **预设配置**：保存当前 Mod 的启用/禁用状态，快速切换不同预设。
- **多游戏切换**：默认支持五款游戏，可自定义添加；每款游戏独立配置 Mod 文件夹与游戏程序路径。
- **启动游戏**：支持原版启动与 EFMI/XXMI 等注入模式，通过全局配置的 XXMI Launcher 拉起。
- **压缩包导入**：内置 zip / 7z / RAR 解压（无需安装 7-Zip / Bandizip），拖入即可导入。
- **Persist Bridge 缓存**：兼容并接管 QAQM 的 Persist Bridge 缓存，无缝迁移。

> 由于继承了 QAQM 的 Mod 结构，可以直接接管 QAQM 已有的配置；Persist Bridge 缓存同样可无缝接管，切换过来无需重建。

---

## 🧱 技术栈

- **Rust**（后端逻辑、解压、扫描、启动、市场 API）
- **Tauri 2**（桌面壳）
- **React 19 + TypeScript + Vite 8**（前端）

---

## 🔨 从源码构建

### 前置要求

- [Rust](https://rustup.rs/)（stable）
- [Node.js](https://nodejs.org/) ≥ 18 + npm
- Windows：WebView2 运行时（Windows 11 已内置；Win10 由安装包自动引导）

### 步骤

```bash
npm install
npm run tauri dev      # 开发模式
npm run tauri build    # 生成安装包（NSIS）
```

构建产物位于 `src-tauri/target/release/bundle/`。

---

## 📦 下载

请前往 [Releases](../../releases) 下载最新安装包（`小林若葉Manager_x.y.z_x64-setup.exe`）。

---

## ⚖️ 许可协议

本项目采用 **GNU General Public License v3.0**（GPL-3.0）开源，详见 [LICENSE](LICENSE)。

本项目为 **QAQM 的复刻/重实现**，UI 与功能设计大量借鉴 QAQM。QAQM 及其中所依赖的第三方组件、游戏 Mod 生态的约定与术语，其权利归各自原作者所有。

---

## 🙏 致谢

- QAQM —— 本项目的设计蓝本与 Mod 结构来源
- [Tauri](https://tauri.app/)、[React](https://react.dev/)、[Vite](https://vite.dev/)
- GameBanana 官方 API
- 各游戏的 Mod 社区与工具链作者

---

## ⚠️ 免责声明

本项目仅用于学习与技术交流，不提供任何游戏资源或 Mod 本体。使用本项目操作 Mod 时，请遵守相关游戏的服务条款与社区规范。
