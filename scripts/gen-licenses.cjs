// 生成第三方组件完整清单（构建期人工运行，产物随应用打包）
const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const srcTauri = "src-tauri";
const outFile = path.join(srcTauri, "resources", "licenses.txt");

// 1) Rust 依赖：cargo metadata 是权威来源（含未下载的 target 依赖）
//    用法：在项目根目录执行 `node scripts/gen-licenses.cjs`
//    依赖变化后需要重新生成，否则清单会过期
let meta;
try {
  const raw = execSync("cargo metadata --format-version 1", {
    cwd: "src-tauri",
    maxBuffer: 128 * 1024 * 1024,
    stdio: ["ignore", "pipe", "ignore"],
  }).toString("utf8");
  meta = JSON.parse(raw);
} catch (e) {
  // 兜底：cargo 不可用时读取手工导出的 src-tauri/target/meta.json
  meta = JSON.parse(fs.readFileSync(path.join("src-tauri", "target", "meta.json"), "utf8"));
}

const pkgs = meta.packages
  .map((p) => ({
    name: p.name,
    version: p.version,
    license: (p.license || "").trim() || "（未声明，见 crates.io）",
    author: (p.authors || []).join(", "),
  }))
  .sort((a, b) => a.name.localeCompare(b.name));

// 2) 前端运行期依赖
const projRoot = ".";
const pkgJson = JSON.parse(fs.readFileSync("package.json", "utf8"));
const feDeps = [];
for (const name of Object.keys(pkgJson.dependencies || {})) {
  try {
    const j = JSON.parse(
      fs.readFileSync(path.join(projRoot, "node_modules", name, "package.json"), "utf8")
    );    feDeps.push({ name, version: j.version, license: j.license || "（未声明）" });
  } catch {
    feDeps.push({ name, version: "?", license: "（未安装）" });
  }
}

const keep = new Set(
  pkgs.filter((p) => /MPL|CDLA|ISC|Unicode|BSD|Zlib|Unlicense|CC0|BSL|0BSD|UnRAR/i.test(p.license))
);

const pad = (s, n) => s + " ".repeat(Math.max(0, n - [...s].reduce((a, c) => a + (c.charCodeAt(0) > 127 ? 2 : 1), 0)));

const lines = [];
lines.push("小林若葉Manager —— 第三方组件完整清单");
lines.push("=".repeat(78));
lines.push("");
lines.push("本清单由 `cargo metadata` 与 node_modules 实测生成，覆盖应用静态链接的全部 Rust 依赖");
lines.push("以及前端打包依赖。分发本应用时请随包附带本文件。");
lines.push("");
lines.push("许可写法说明：");
lines.push("  A OR B   = 双许可，使用方可任选其一遵守");
lines.push("  A AND B  = 需同时满足多个许可");
lines.push("  MPL-2.0  = 文件级弱著佐权；本应用未修改这些组件，只需保留其许可声明");
lines.push("");
lines.push("本应用不含 GPL / AGPL 等强著佐权组件。");
lines.push("");
lines.push("=".repeat(78));
lines.push(`【一】Rust 依赖（静态编译进程序，用户无需安装）  共 ${pkgs.length} 个`);
lines.push("=".repeat(78));
lines.push(pad("组件", 34) + pad("版本", 14) + "许可证");
lines.push("-".repeat(78));
for (const p of pkgs) {
  lines.push(pad(p.name, 34) + pad(p.version, 14) + p.license);
}
lines.push("");
lines.push("=".repeat(78));
lines.push(`【二】前端依赖（打包进 dist/）  共 ${feDeps.length} 个`);
lines.push("=".repeat(78));
lines.push(pad("组件", 34) + pad("版本", 14) + "许可证");
lines.push("-".repeat(78));
for (const p of feDeps) {
  lines.push(pad(p.name, 34) + pad(p.version, 14) + p.license);
}
lines.push("");
lines.push("构建工具（Vite / TypeScript / @vitejs/plugin-react / @types/*）仅在开发与打包阶段使用，");
lines.push("其代码不进入发行产物。");
lines.push("");
lines.push("=".repeat(78));
lines.push("【三】UnRAR 声明（必须保留）");
lines.push("=".repeat(78));
lines.push("");
lines.push("本应用的 RAR 解压能力通过 unrar / unrar_sys crate 静态链接 UnRAR C++ 源码实现，");
lines.push("仅使用解压与列表功能，未实现任何 RAR 压缩功能。");
lines.push("");
lines.push("UnRAR source code may be used in any software to handle RAR archives");
lines.push("without limitations free of charge, but cannot be used to develop RAR");
lines.push("(WinRAR) compatible archiver and to re-create RAR compression algorithm,");
lines.push("which is proprietary.");
lines.push("");
lines.push("Copyright (C) 1993-2024 Alexander Roshal. All copyrights to RAR and the");
lines.push("utility UnRAR are exclusively owned by the author - Alexander Roshal.");
lines.push('UnRAR 以 "AS IS" 形式提供，作者不对数据丢失、损害或利润损失承担任何责任。');
lines.push("");
lines.push("=".repeat(78));
lines.push("【四】解压能力说明");
lines.push("=".repeat(78));
lines.push("");
lines.push("zip  → zip crate（MIT，纯 Rust）");
lines.push("7z   → sevenz-rust2 crate（Apache-2.0 OR MIT，纯 Rust）");
lines.push("rar  → unrar crate（静态链接 UnRAR 源码）");
lines.push("");
lines.push("本应用不包含、也不调用 7-Zip 的 7z.exe / 7z.dll，用户无需安装第三方解压软件。");
lines.push("");
lines.push("=".repeat(78));
lines.push("【五】数据与素材");
lines.push("=".repeat(78));
lines.push("");
lines.push("GameBanana：Mod 列表、详情、评论、分类与下载链接来自其公开 API");
lines.push("（https://gamebanana.com），使用时应遵守其服务条款与 API 规范。");
lines.push("");
lines.push("各游戏的角色立绘、游戏图标等素材版权归各自游戏发行商所有，本应用仅作识别与展示用途。");
lines.push("");
lines.push("=".repeat(78));
lines.push("【六】设计参考（未包含其源代码）");
lines.push("=".repeat(78));
lines.push("");
lines.push("IMM — Integrated Mod Manager（https://github.com/jpbhatt21/integrated-mod-manager）");
lines.push("该仓库未附带开源许可证，因此本项目未复制其代码，仅在功能行为与界面组织上作参考。");
lines.push("");
lines.push("QAQM：同类模组管理器，仅作功能与交互参考；本应用继承了其 Mod 目录结构以便用户迁移配置。");
lines.push("");
lines.push("=".repeat(78));
lines.push("生成信息");
lines.push("=".repeat(78));
lines.push(`生成时间: ${new Date().toISOString()}`);
lines.push(`Rust 依赖: ${pkgs.length} 个   前端依赖: ${feDeps.length} 个`);
lines.push("");
lines.push(`特殊许可组件 ${keep.size} 个（MPL / CDLA / ISC / BSD / Zlib / Unicode / Unlicense 等）：`);
for (const p of [...keep].sort((a, b) => a.name.localeCompare(b.name))) {
  lines.push(`  ${pad(p.name, 30)}${pad(p.version, 12)}${p.license}`);
}

fs.mkdirSync(path.dirname(outFile), { recursive: true });
fs.writeFileSync(outFile, lines.join("\r\n"), "utf8");
console.log(`已生成 ${outFile}`);
console.log(`Rust ${pkgs.length} 个 / 前端 ${feDeps.length} 个 / 特殊许可 ${keep.size} 个`);
