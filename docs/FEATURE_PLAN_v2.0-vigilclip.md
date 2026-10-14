# VigilClip（v2.0.0）改造方案

由 PastePaw 改名而来。需求全部来自 2026-10-12 的一次对话，决策由用户逐条确认。

## 1. 已确认决策

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 改名范围 | **全量**：crate / exe / 仓库名 / productName / identifier / 数据目录 |
| 2 | identifier | 改为 `com.vigilclip.app`，数据目录改为 `VigilClip` |
| 3 | 数据迁移 | **自动迁移**：启动时发现旧目录就搬过去，先搬后校验，不做删除式操作 |
| 4 | GitHub 仓库名 | 由用户在仓库 Settings 改为 `VigilClip`；代码侧同步所有链接、CI、更新检查地址 |
| 5 | 图标 | 多层卡片错位堆叠 + 渐变，配色紫→青 |
| 6 | 设置界面 | 参照 Paste 重排信息架构：左侧窄导航 + 右侧单列行式设置 |
| 7 | 语言 | 英文翻译补齐（去掉硬编码中文），**默认语言 = 中文** |
| 8 | 字体 | 下拉列出本机所有已安装字体（UI 字体 + 内容字体都用全量列表） |
| 9 | 文件夹筛选 | 彩色圆点 + 名字（像 Paste） |
| 10 | 卡片悬停 | 小幅度放大动画 |

## 2. 改名清单（PastePaw → VigilClip）

### 后端
- `src-tauri/Cargo.toml`：包名 `PastePaw` → `vigilclip`（cue: `main.rs` 的 `use PastePaw::run_app`、`lib.rs` 的 `#![allow(non_snake_case)]` 注释）
- `src-tauri/tauri.conf.json`：`productName`、`identifier`、`app.windows[0].title`、asset 协议作用域 `$DATA/PastePaw/images/**`
- `src-tauri/tauri.appstore.conf.json`：窗口标题（v2.0.0 之后移除了对上游 provisioning profile 的引用）
- 数据目录：`get_data_dir()` 返回 `…\VigilClip`；库文件 `paste_paw.db` → `vigilclip.db`；图片目录 `…\VigilClip`
- 托盘：tooltip、id 相关字符串
- 开机自启：`tauri-plugin-autostart` 的注册名（identifier 变则注册表项变，需重新登记）
- 单实例：互斥名/窗口类名
- `export.rs`：导出文件的 `app` 字段改为 `VigilClip`；**导入仍要接受旧的 `PastePaw` 导出文件**（按 `version` 校验，不按 `app`）
- `updater.rs`：`RELEASES_API` / `RELEASES_PAGE` 指向新仓库名；User-Agent
- `capabilities/default.json`：opener 白名单的三个 URL

### 前端
- 界面里所有 "PastePaw" 文案（i18n 各语言文件、关于区域、欢迎/空状态）
- `localStorage` 键 `pastepaw_language` → `vigilclip_language`（读旧键做一次兼容读取）
- 默认语言 `en` → `zh`（`AppSettings::default()` + 前端默认值）

### CI / 文档
- `.github/workflows/release.yml`：产物名、便携版收集路径 `PastePaw.exe` → `VigilClip.exe`、release 名称
- `README.md` / `README_CN.md`：标题、徽章、图标 URL、下载产物名、仓库链接
- `AGENTS.md`、`docs/*`：项目名
- 6 张截图：重拍（界面里的产品名变了）

## 3. 数据迁移（唯一有数据风险的部分）

```
%APPDATA%\PastePaw\paste_paw.db      →  %APPDATA%\VigilClip\vigilclip.db
%APPDATA%\PastePaw\images\           →  %APPDATA%\VigilClip\images\
%APPDATA%\me.xueshi.pastepaw\settings.json → %APPDATA%\com.vigilclip.app\settings.json
```

规则：
1. **只在目标不存在时迁移**，不覆盖任何东西。
2. 优先用 `fs::rename`（同盘 = 原子移动，源目录随之消失，**没有删除步骤**）。
3. `rename` 跨盘失败时退化为复制，复制后校验（文件数 + 字节数），成功才移除源文件；校验失败则保留源、报错并继续启动一个新库（不静默丢数据）。
4. 迁移结果写 info 日志，并在设置界面显示一次性提示（可选）。
5. 单元测试覆盖：目标已存在时不动、迁移后行数一致、源已被移走。

## 4. 图标规格

- 设计：圆角方块底（底色 `#1B1146`→`#2B1A6B`），三张卡片错位堆叠
  - 卡片1 `#7C5CFF`→`#A98BFF`，卡片2 `#4F8CFF`→`#6FD0FF`，卡片3 `#2AD3B8`→`#7CF0D8`
- 产出：`32x32.png`、`64x64.png`、`128x128.png`、`128x128@2x.png`、`icon.ico`（16/24/32/48/64/128/256）、`Square*Logo.png` 全尺寸、`StoreLogo.png`
- 托盘：`tray.png`（彩色）+ `tray_white.png`（单色剪影，随主题切换）
- 竖屏缩到 16px 仍要能认出"叠着的卡片"→ 16px 版减少细节（卡片只留 2 层、去掉描边）
- 生成方式：Python + Pillow 脚本（`tools/make_icon.py`，提交进仓库以便日后改色重生成）

## 5. 设置界面信息架构（参照 Paste）

左侧窄导航 + 右侧单列行式，窗口尺寸从当前的设置窗口尺寸微调（需要实测行高）。

| 分区 | 行 |
|------|----|
| 通用 | 主题、语言、显示大小、文字大小 |
| 外观 | 窗口效果、圆角、悬浮于任务栏之上、界面字体/字重、内容字体/字重 |
| 行为 | 开机自启、自动粘贴、粘贴方式、忽略幽灵副本、全局快捷键 |
| 文件夹 | 文件夹列表（含颜色圆点）、新建、改名、删除 |
| 数据 | 保留天数、条数上限、立即清理、导出、导入 |
| 隐私 | 忽略的应用列表 |
| 关于 | 版本、检查更新、开源许可 |

- 每行：左标题（+ 次要说明）右控件，行间 1px 分隔线，去掉现在层层嵌套的卡片与重复标题
- 保持同一套 Tailwind token（`bg-card`/`border`/`muted-foreground`），不引入新依赖

## 6. 字体枚举

后端新增 `get_installed_fonts() -> Vec<String>`：
- 用 GDI `EnumFontFamiliesExW` 枚举字体族（比读注册表准：注册表给的是显示名，且含大量非 UI 用途条目）
- 过滤以 `@` 开头的竖排变体，去重，按名称排序
- 结果缓存（`OnceLock`），避免每次打开设置都枚举
- 前端：两个字体下拉改成"默认 + 全量列表"，当前值不在列表里时补一条（避免显示空白）

## 7. 文件夹彩色圆点

- `folders.color` 字段已存在但从未写入（`create_folder` 一直传 `null`）
- 圆点颜色：`folder.color` 优先，为空时按名字哈希从固定调色板取色（沿用 ControlBar 现有算法）
- 文件夹弹窗增加颜色选择（8-10 个预设色 + "自动"）
- 彩色圆点只在文件夹筛选器出现（ClipCard 上的文件夹标签保持现在的小图标样式）

## 8. 卡片悬停动画

- 现在：`hover:-translate-y-1 shadow-lg`，选中 `scale-[1.02]`
- 增加：悬停 `scale-[1.03]` + 阴影加深 + `duration-150`（窗口高 266px，缩放 3% 不会溢出容器被裁）
- 尊重 `prefers-reduced-motion`：该偏好下不加缩放

## 9. 验收标准

| 项目 | 标准 |
|------|------|
| 迁移 | 旧库 1026 条历史、文件夹、设置、忽略应用列表全部出现在新版本；旧目录不再被引用 |
| 迁移幂等 | 重复启动不重复迁移、不覆盖已有新数据 |
| 改名完整 | 全仓库 grep `PastePaw` 只剩：迁移代码里的旧路径常量、CHANGELOG 历史、GPL 归属说明、文档中说明改名历史的部分 |
| 字体 | 下拉条数 ≥ 系统字体数（本机实测约 200+），当前值回显正确 |
| 圆点 | 每个文件夹都有颜色，深色/浅色主题下都清楚 |
| 悬停 | 悬停有放大过渡，175ms 内完成，不遮挡相邻卡片 |
| 设置界面 | 左侧导航 + 右侧行式；6 个分区都能进入；所有控件功能与改版前一致 |
| 英文 | 切到 English 后界面无中文残留（字体名、语言名除外） |
| 图标 | 16px 下仍可辨认；托盘深浅主题都清楚 |

## 10. 执行顺序

```
1. 数据迁移模块 + 测试        ← 先做，纯后端、可测，风险最高
2. 全量改名（后端 + 配置）    ← 依赖 1
3. 字体枚举命令              ← 独立
4. 前端：设置界面重排 + 字体下拉 + i18n 补齐 + 默认中文
5. 前端：文件夹圆点 + 悬停动画
6. 图标生成 + 全套替换
7. CI / README / 文档 / 截图重拍
8. 发版 v2.0.0
```

版本定 v2.0.0：identifier、数据目录、仓库名、crate 名全变，属于身份级变更，按语义化版本应给大版本。
