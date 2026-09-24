# PastePaw 功能方案 v1.6 – v1.8

> 分支：个人分支（不对齐上游发布）
> 基线：v1.5.0
> 状态：**v1.6 进行中**

## 进度（分支 `feat/v1.6-multi-format-clipboard`）

| 步骤 | 内容 | 状态 |
|---|---|---|
| 1 | `clip_formats` 表 + `CF_HDROP`/`CF_HTML` 编解码器 | ✅ 完成 |
| 2 | `CF_HDROP` 文件捕获（需求 2） | ✅ 完成 |
| 3 | 富文本 `CF_HTML`/`CF_RTF` 捕获与回写（需求 5-③） | ✅ 完成 |
| 4 | 原生回写：文件 / 文本 / 图片（修 BUG-3） | ✅ 完成 |
| — | 文件卡片渲染（修卡片显示完整路径 / “N characters”） | ✅ 完成 |
| 5 | 保留策略 + 设置 UI（需求 4，修 BUG-1） | ✅ 完成 |
| 6 | `P` 键接线 + 内置收藏文件夹（修 BUG-2） | ✅ 完成 |

**v1.6 已完成**（需求 2、需求 4、需求 5-③ + BUG-1/2/3）。

### v1.6 + v1.7 实际产出

| 指标 | 之前 | 现在 |
|---|---|---|
| Rust 单元测试 | 0 | **59** |
| Rust 源文件 | 11 | 15（新增 `clipboard_formats.rs`、`retention.rs`、`pins.rs`、`export.rs`） |
| 新增数据库表 | — | `clip_formats` |

### v1.7 进度：文件夹导出 / 导入（需求 1）

| 项 | 状态 |
|---|---|
| 导出：单个 JSON，含全部文件夹（D-05） | ✅ 完成 |
| 导入：文件夹重名合并、内容 hash 相同跳过、图片缺失保留并标记（D-04） | ✅ 完成 |
| 设置页入口 | ✅ 完成 |
| 验收：可人工阅读、可手改后仍能导入 | ✅ 已由测试覆盖 |
| 验收：内容 hash 幂等（连导两次 == 导一次） | ✅ 已由测试覆盖 |
| 验收：1000 条 + 100 张图片导出 ≤ 5s | ✅ **实测 12.09 ms**（预算 5s） |

**JSON 形状**（导出用 `to_string_pretty`，字段名即下方名称）：

```json
{
  "version": 1,
  "app": "PastePaw",
  "exported_at": "2026-09-24T12:00:00+00:00",
  "folders": [{ "name": "work", "icon": null, "color": null, "is_system": false }],
  "clips": [{
    "clip_type": "text",
    "content": "…",
    "text_preview": "…",
    "content_hash": "…",
    "folder": "work",
    "source_app": "chrome.exe",
    "metadata": null,
    "created_at": "2026-09-24T11:59:00+00:00",
    "html": "<b>…</b>",
    "rtf_base64": null,
    "image_path": null
  }]
}
```

文件夹用 **名称** 引用而非 id（id 只在产生它的数据库里有意义）。
`file` 类型的路径列表由换行拼接的 `content` 重建，不在 JSON 里存两份。

**尚未验证（最大风险）**：以上全部功能均**未做端到端人工验证**。
单元测试覆盖的是纯字节布局、数据库与保留策略逻辑；
**真实的 Win32 剪贴板读写路径、以及窗口动画，没有任何自动化覆盖**，
需要应用运行 + 人手动复制/粘贴才能确认。

> 另：本次会话中途机器的 **Device Guard / 智能应用控制** 开始拦截一切新编译的
> 可执行文件（CodeIntegrity 事件 3077，策略 ID `{0283ac0f-…}`），
> 一度使 `cargo test` 与 `cargo tauri dev` 都无法运行。
> 已由用户关闭智能应用控制解除（**该操作不可逆**，需重装系统才能重新开启）。

**已知未处理**：
- 若某个来源只提供 `CF_HTML` 而**没有**纯文本，该剪贴不会被记录（与改动前一致，无回退），仅记日志。
- `extract_icon`（`clipboard.rs`）的 `chunks_exact_mut` 可改为 `as_chunks_mut`（既存 clippy 告警，非本次引入）。

---

## 0. 已确认决策（2026-09 定稿）

| 编号 | 决策 | 结论 |
|---|---|---|
| **B-01** | 导出时图片怎么办 | ✅ **方案 乙** —— 只导路径，**接受换设备后图片全部丢失** |
| D-02 | 保留天数档位 | ✅ `1 / 7 / 30 / 90 / 365 / 永久`，默认 30 |
| D-03 | `max_items` 死设置 | ✅ 一并实现，与天数取「先到先删」 |
| D-04 | 导入冲突策略 | ✅ 内容 hash 相同→跳过；文件夹重名→合并；图片缺失→保留并标记 |
| D-05 | 导出粒度 | ✅ 单个 JSON，含全部文件夹 |
| D-06 | i18n 范围 | ✅ 新文案只做 en + zh，ja/fr/de 英文兜底 |
| D-07 | 富文本范围 | ✅ `CF_HTML` + `CF_RTF` 都要 |

### B-01 结论的已知代价（已接受）

导出 JSON 中图片字段为**绝对路径**（如 `C:\Users\Mechrevo\...\abc.png`）。
在另一台电脑上导入后，**所有图片条目的图片文件均不存在**，将显示为「图片缺失」占位符。
文本类数据（text / html / rtf / file 路径）**不受影响**。

---

## 0.1 原始阻塞项记录（历史）

### ⚠️ B-01 你的 Q2 选择互相冲突：图片换设备后必然失效

你在 Q2 里的四个选择：

| 你的选择 | 后果 |
|---|---|
| 1. 只导出文件夹里的 | ✅ 可行 |
| 2. 只给 JSON | ✅ 可行 |
| 3. 图片只导出**路径** | ⚠️ 路径指向**原机器的磁盘** |
| 4. 另一台设备要**能被 PastePaw 导入** | ❌ 与第 3 条冲突 |

**冲突原因**：JSON 里存的是 `C:\Users\Mechrevo\...\PastePaw\images\abc.png`。
这个路径在另一台电脑上**不存在**，导入后所有图片条目都会变成"图片缺失"。

**三个选项，请选一个：**

- **方案 甲（推荐）**：JSON 里图片写**相对路径**（`images/abc.png`），导出时把图片一起复制到 `导出目录/images/`。
  → 仍是"纯 JSON 文件"，旁边多一个图片文件夹。导入时按相对路径找回图片。**跨设备图片不丢。**
- **方案 乙**：只导路径，接受换设备后图片全丢（图片变成占位符）。
  → 最省事，但你的需求 1「让其他设备直接使用我的数据」对图片不成立。
- **方案 丙**：导出成 zip 包（含 manifest.json + images/）。
  → 最干净，但你 Q2 明确否决了非 JSON 格式。

---

## 1. 已发现的问题（本轮范围内需要一并处理）

| 编号 | 问题 | 证据 | 影响 |
|---|---|---|---|
| **BUG-1** | `auto_delete_days` / `max_items` 是**死设置** | 全仓仅在 `models.rs`（默认值）和 `settings_manager.rs`（读取）出现，**无任何清理逻辑**，`SettingsPanel.tsx` 也无 UI | 历史无限增长，磁盘无限膨胀。你 Q7 要求实现保留天数，必须一并修 |
| **BUG-2** | `P` 键 Pin/Unpin **完全无效** | `useKeyboard.ts:31` 支持 `onPin`，但 `App.tsx:569-576` 调用 `useKeyboard` 时**没有传 `onPin`**。README 和 AGENTS.md 都把它写成了可用快捷键 | 文档与实现不符 |
| **BUG-3** | 图片粘回**有损** | `commands.rs:521-523` 注释 `// Frontend writes image via navigator.clipboard API`，Rust 侧不写图片 | 只写 PNG 到浏览器剪贴板，**没有 `CF_DIB`/`CF_DIBV5`**。部分应用（Word、画图、部分国产软件）粘贴会失败。这是 Q1 验收「图片像素级一致」的必经修复 |

---

## 2. 对标 Paste 的等价物映射（Q1 确认：行为对齐 + Windows 原生等价实现）

| Paste (macOS) | Windows 等价物 | 现状 | 目标 |
|---|---|---|---|
| `NSPasteboard` 保留原始格式 | 多格式捕获（见下表） | 只存 text / image | v1.6 |
| 粘贴按原格式回写 | 回写同格式集 | text 走插件；image 走前端（有损） | v1.6 |
| 文件存路径引用（`fileURL`） | `CF_HDROP` + `DROPFILES` 结构 | 无 | v1.6 |
| 富文本原样保留 | `CF_HTML`（"HTML Format"）+ `CF_RTF` | 无 | v1.6 |
| **Keep History** 保留天数，默认 30 天，**已 Pin 的条目豁免** | 同语义 | 有字段无实现 | v1.6 |
| Pinboard（收藏集合，永久保留） | **已有「文件夹」**，语义已等价 | ✅ 已有 | v1.8 补齐差异 |
| Core Animation 合成层动画 | DWM + 前端 CSS `transform` 合成层 | Rust 逐帧 `SetWindowPos` | v1.8 |
| **导出/导入历史** | —— | —— | **Paste 官方没有此功能** |

> **注**：Paste 官方**不提供**导出/导入（只靠 iCloud 同步；用户在 pasteapp.nolt.io/39 提过需求，未实现）。
> 所以 v1.7 没有对标对象，JSON 格式由本项目自定。

---

## 3. 版本规划

### v1.6 —— 多格式剪贴板底层 + 保留策略
**对应需求**：需求 2（复制图片/文件）、需求 4（保留天数）

**范围**
1. **多格式捕获**：新增 `clip_formats` 表，一条 clip 可挂多个格式
   - `CF_UNICODETEXT` → text
   - `CF_HTML` → html
   - `CF_RTF` → rtf
   - `CF_DIBV5` / `CF_PNG` → image
   - `CF_HDROP` → file（路径列表，**不复制文件本体**）
2. **原生回写**：`paste_clip` 改为 Rust 侧 `OpenClipboard` + 逐格式 `SetClipboardData`
   - 修复 BUG-3（图片从有损改为无损）
   - 文件走 `CF_HDROP`，资源管理器可直接「粘贴」
3. **保留策略**（对齐 Paste Keep History）
   - 设置 UI：`1 / 7 / 30 / 90 / 365 / 永久`，默认 **30 天**
   - 清理逻辑：`folder_id IS NULL` 的条目才过期；**文件夹内条目永久保留**
   - 连带清理 `clip_images` 磁盘文件（避免 BUG-1 类泄漏）
   - 触发时机：启动时 + 新剪贴入后（低频，不阻塞） + 手动「立即清理」
   - 一并处理 `max_items`
4. **前端**：新增 file / html / rtf 卡片预览渲染
5. 修复 **BUG-2**（`App.tsx` 接线 `onPin`）

**验收**
| 指标 | 目标 |
|---|---|
| 图片粘回 | 像素级一致（hash 相等），Word / 画图 均可粘贴 |
| 文件粘回 | 资源管理器可粘贴，多选 + 文件夹均可用 |
| 富文本粘回 | Word 中保留粗体/斜体/超链接 |
| 保留天数 | 改设置后，过期条目在下次清理时消失，磁盘图片同步删除 |
| 文件夹豁免 | 文件夹内条目**永不被**保留策略删除 |
| 呼出延迟 | ≤ 80ms（P95），**不因多格式捕获而退化** |

**风险**
- `CF_HTML` 的 "HTML Format" 头（`Version` / `StartHTML` / `EndHTML` / `StartFragment` 字节偏移）必须严格正确，否则目标应用粘贴出乱码 → 需按 MS 规范构造并做边界测试
- 多格式存储会让 DB 体积显著增大（同内容存 text + html + rtf）→ 需设单条体积上限 + 去重

---

### v1.7 —— 文件夹数据导出 / 导入
**对应需求**：需求 1

**范围**
1. **导出**（格式待 0 节确认）
   - 单文件 JSON：`{ version, exported_at, folders[], clips[] }`
   - 范围：**仅文件夹内条目**（你 Q2 选择）
   - 图片：按 0 节选定方案处理
2. **导入**
   - 冲突策略（**D-04，待确认**）
3. **UI**：设置页入口 + 导出/导入进度提示
4. i18n：仅 en + zh（**D-06，待确认**）

**验收**
| 指标 | 目标 |
|---|---|
| 导出 | 生成的 JSON 可被人工阅读、可手工编辑后仍能导入 |
| 导入还原 | 文件夹名/图标/颜色/条目归属 100% 还原 |
| 幂等性 | **同一份 JSON 连续导入两次，结果与导入一次相同**（不产生重复） |
| 大数据 | 1000 条 + 100 张图片的导出 ≤ 5s |

---

### v1.8 —— 流畅度 + 体验功能
**对应需求**：需求 3（流畅度）、需求 5（其他功能 ①③④⑤⑥）

**范围**
1. **动画架构重构**（需求 3）
   - 先做 **spike 原型验证**，再决定是否全量改
   - 目标：从 Rust 逐帧 `SetWindowPos` → 前端 CSS `transform` 合成层
   - 保留 shader 语义：`IS_ANIMATING` 原子锁 + `LAST_SHOW_TIME` 500ms 防抖 + 滑到任务栏后 Z-order 下沉
2. **内容类型标签页**（①）：文本 / 图片 / 文件 / 链接 / 颜色 筛选
3. **Pinboard 差异补齐**（④）
   - ⚠️ 现有「文件夹」**已经是** Paste 的 Pinboard（都永久保留）
   - 建议**不新建概念**，只补：搜索结果显示所属列表 + 「跳转到所在列表」
4. **按源应用筛选/分组**（⑤）
5. **`Ctrl+1~9` 快速粘贴**（⑥）
6. **不做**：② 拖拽卡片到外部应用（你 Q4 决定）

**验收**
| 指标 | 目标 |
|---|---|
| 呼出/隐藏动画 | ≥ 55fps |
| 1000 条滚动 | ≥ 55fps，无掉帧 |
| 搜索输入 → 刷新 | ≤ 50ms |
| 热键 → 可见 | ≤ 80ms（P95） |

**风险**
- **R1 动画重构是架构级改动**。Tauri 窗口动画受 DWM 合成控制，改法可能反而更卡或撕裂。**必须先 spike，失败即回退**到优化现有逐帧逻辑（减少帧数 / 只重绘脏区）。
- **R2 回归风险**：`lib.rs` 的动画状态机是 issue #6/#9/#10 的修复点，改动需重跑这些场景（多显示器、混合 DPI、睡眠唤醒、锁屏）。

---

## 4. 待确认决策点

| 编号 | 决策 | 建议值 |
|---|---|---|
| **阻塞** | **B-01** 导出时图片怎么办 | **方案 甲**：相对路径 + 旁挂 `images/` 目录 |
| D-02 | 保留天数选项值（Paste 未公布确切档位） | `1 / 7 / 30 / 90 / 365 / 永久`，默认 30 |
| D-03 | `max_items`（条数上限）这个死设置怎么办 | 一并实现（与天数取"先到先删"） |
| D-04 | 导入冲突策略 | 内容 hash 相同 → **跳过不建副本**；文件夹重名 → **合并**；图片文件缺失 → 条目保留并标记"图片缺失" |
| D-05 | 导出粒度 | **单个 JSON**，内含全部文件夹（非每文件夹一个文件） |
| D-06 | i18n 范围（个人分支） | 新文案只做 **en + zh**，ja/fr/de 走英文兜底 |
| D-07 | 富文本格式范围 | **`CF_HTML` + `CF_RTF` 都要**（与 Paste/Maccy 一致） |

---

## 5. 执行顺序（Q5 确认）

```text
v1.6  多格式底层 + 保留策略   ← 风险中，价值最高，先做
  └ 内含 BUG-1 / BUG-2 / BUG-3 修复
v1.7  导出 / 导入             ← 依赖 v1.6 的文件夹+多格式结构
v1.8  动画重构 + 体验功能      ← 最高风险，先 spike
```

理由：v1.6 的多格式底层（`CF_HDROP` + `CF_HTML` + `CF_RTF` + 原格式回写）
正是 v1.7 导出和 v1.8 类型标签页的共同基础。**一次改完，比做两遍便宜得多。**

---

## 6. 开发环境（已搭建，2026-09-24）

### 已安装（本机此前完全没有 Rust 工具链）

| 组件 | 版本 | 路径 |
|---|---|---|
| rustup / cargo / rustc | 1.98.1 | `C:\Users\Mechrevo\.cargo\bin` |
| toolchain | `stable-x86_64-pc-windows-msvc` | `~/.rustup` |
| VS BuildTools 2022 + C++ 工作负载 | 17.14.37710.0 | `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools` |
| MSVC 工具集 | 14.44.35207 | `...\VC\Tools\MSVC\14.44.35207` |
| Windows SDK | 10.0.26100.0 | `C:\Program Files (x86)\Windows Kits\10` |
| Node / pnpm | 24.13.0 / 12.6.0 | 已有 |

### 每次跑 cargo 都必须先设 PATH

`cargo` 不在系统 PATH 中，且 **Git Bash 自带的 `C:\Program Files\Git\usr\bin\link.exe` 会遮蔽 MSVC 链接器**，
所以必须显式把 MSVC 的 `Hostx64\x64` 目录前置：

```bash
export PATH="/c/Users/Mechrevo/.cargo/bin:/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64:$PATH"
export CARGO_TERM_COLOR=never
```

### 前置条件：必须先构建前端

`tauri.conf.json` 的 `frontendDist: "../dist"` 指向 `F:\develop\PastePaw\dist`。
**该目录不存在时 `cargo check` 会失败**：

```
error: proc macro panicked  (src/lib.rs:362)
  = help: message: The `frontendDist` configuration is set to `"../dist"` but this path doesn't exist
```

解决：先跑 `pnpm build`（= `tsc && vite build`，输出到 `dist/`）。

### 基线验证结果（未修改源码）

| 检查 | 命令 | 结果 |
|---|---|---|
| 前端构建 | `pnpm build` | ✅ 通过（输出 634 KB JS） |
| 类型检查 | `pnpm exec tsc --noEmit` | ✅ 通过 |
| Rust 检查 | `cargo check` | ✅ 通过 |
| Lint | `cargo clippy --all-targets` | ✅ 通过，6 个既有 warning |
| 测试 | `cargo test` | ✅ 通过，**但 0 个测试** |

### 环境相关的坑（已踩过）

| 现象 | 真因 | 处理 |
|---|---|---|
| `winget install ... 2022.BuildTools` 装完却没有 C++ | winget 默认不装任何工作负载 | 用 `setup.exe modify --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended` |
| `setup.exe modify` 退出码 87 | **`--wait` 不是 `setup.exe modify` 支持的参数** | 去掉 `--wait`，改轮询 `VC\Tools\MSVC` 出现 |
| `cmd.exe /c start ...` 启动成交互模式 | Git Bash 的 MSYS 把 `/c` 转成 `C:/` | 用 PowerShell `Start-Process` 分离启动 |
| `cargo check` 报 `tauri-plugin-fs` 构建脚本 `os error 4551` | 瞬时故障（当时 VS 安装的 msiexec 未退出），非 SAC 拦截 | 重跑即通过 |
| 新编译的 exe 在 Git Bash 里报 `Permission denied` | MSYS 假象，**不是** Smart App Control | 用 `cmd.exe /c` 执行即为正常 |

> ⚠️ 本机 **Smart App Control 处于强制状态**（`VerifiedAndReputablePolicyState = 1`）。
> 本次未触发拦截，但若后续出现「应用程序控制策略已阻止此文件」，根因即在此时。
> 注意：关闭 Smart App Control 是**不可逆**的（需重装系统才能恢复），必须先评估。

### ⚠️ 测试覆盖为零

`cargo test` 跑通但 **0 个测试**。v1.6 的 `CF_HTML` 字节偏移写入
（`StartHTML`/`EndHTML`/`StartFragment`）是极易出错且难排查的部分，
**建议同步补 Rust 单元测试**，否则只能靠手工粘到 Word 验证。
