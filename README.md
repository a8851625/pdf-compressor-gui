# PDF Compress GUI

跨平台 PDF 压缩桌面应用（**Tauri + Svelte**），压缩核心为 **Ghostscript**，支持 macOS / Windows 打包。

> 当前策略：**构建时内置 Ghostscript**（优先使用 app resources 内置 GS，找不到再回退系统 PATH）。
>
> 核心压缩逻辑参考：[`theeko74/pdfc`](https://github.com/theeko74/pdfc)

---

## 功能特性

- 拖拽或选择 PDF 文件
- 压缩档位选择（0 ~ 4）
- 输出路径选择
- 压缩前后体积和变化比例展示
- 压缩过程 loading + 进度条 + 耗时提示
- 清晰错误提示（路径无效、权限不足、Ghostscript 不可用等）

---

## 技术栈

- 前端：Svelte + Vite
- 桌面容器：Tauri v2
- 后端：Rust（命令调用 Ghostscript）
- 打包：`.dmg`（macOS）、`.msi/.exe`（Windows）

---

## 项目结构

```text
.
├─ src/                         # Svelte 前端
├─ src-tauri/
│  ├─ src/main.rs               # Rust 命令与 Ghostscript 调用
│  ├─ tauri.conf.json           # Tauri 配置
│  ├─ capabilities/             # Tauri v2 权限
│  ├─ icons/                    # 应用图标
│  └─ resources/ghostscript/    # 构建时注入的内置 GS（自动生成）
├─ scripts/embed-ghostscript.mjs# 构建时内置 GS 脚本
└─ .github/workflows/           # CI 打包工作流
```

---

## 环境要求

- Node.js 18+
- npm 9+
- Rust stable
- 平台对应 Tauri 依赖（见官方文档）

---

## 本地开发

```bash
npm install
npm run tauri:dev
```

说明：
- `tauri:dev`/`tauri:build` 前会自动运行 `npm run embed:gs`
- 会将 Ghostscript 复制到 `src-tauri/resources/ghostscript`

---

## 打包构建

```bash
npm run tauri:build
```

产物位置（示例）：
- `src-tauri/target/release/bundle/dmg/*.dmg`
- `src-tauri/target/release/bundle/msi/*.msi`
- `src-tauri/target/release/bundle/nsis/*.exe`

---

## Ghostscript 内置策略

### macOS

`embed-ghostscript.mjs` 按以下顺序查找 gs：
1. `GS_MAC_PATH`
2. `/opt/homebrew/bin/gs`
3. `/usr/local/bin/gs`
4. `which gs`

并复制：
- `gs` 可执行文件
- 依赖 `.dylib`（自动修复 install_name）
- `share/ghostscript`

可显式指定：

```bash
GS_MAC_PATH=/opt/homebrew/bin/gs npm run tauri:build
```

### Windows

脚本查找顺序：
1. `GS_WIN_DIR`
2. `C:\Program Files\gs\gs*`
3. `C:\Program Files (x86)\gs\gs*`

找到后复制整个目录到：
- `src-tauri/resources/ghostscript/windows`

可显式指定：

```powershell
$env:GS_WIN_DIR = "C:\Program Files\gs\gs10.07.0"
npm run tauri:build
```

---

## 运行时 Ghostscript 选择顺序

Rust 后端优先顺序：
1. App resources 内置 GS（平台路径）
2. 系统 PATH 中的 `gs` / `gswin64c` / `gswin32c`

> 如果内置可用，会优先使用内置版本。

---

## GitHub Actions 打包

仓库已提供 CI 工作流：
- `.github/workflows/release-build.yml`

默认行为：
- macOS runner：通过 Homebrew 安装 Ghostscript 后打包
- Windows runner：通过 Chocolatey 安装 Ghostscript 后打包
- 生成各平台安装包并上传 artifacts

触发方式：
- `push` 到 `main`
- `workflow_dispatch` 手动触发

---

## 常见问题（FAQ）

### 1) 报错“未找到可用的 Ghostscript”

请按顺序检查：
1. 构建日志是否有 `✅ 已内置 Ghostscript`
2. 包内是否存在 `resources/ghostscript/...`
3. 系统 GS 是否可执行（仅回退场景）

### 2) 为什么打包体积变大？

因为将 Ghostscript 及其依赖直接内置进安装包，这是“开箱即用”的代价。

### 3) 新初始化构建环境能否直接构建？

可以，只要构建机能安装 Ghostscript（或设置到可发现路径）。
CI 工作流已覆盖这一步。

---

## 许可证与合规提醒

Ghostscript 采用 AGPL / 商业授权双许可。对外分发前，请确认你的分发与闭源策略符合其授权要求。
