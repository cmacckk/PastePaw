# Changelog

All notable changes to PastePaw will be documented in this file.

## v1.6.0

### Added
- **Files on the clipboard**: Copying files or folders records their paths, and pasting puts them back so File Explorer accepts a normal paste.
- **Rich text**: Formatting, links and tables are captured as HTML and RTF, so pasting into a word processor keeps them instead of flattening to plain text.
- **History retention**: Choose how many days and how many items to keep. Items saved to a folder are exempt from both limits. These settings existed but were never applied.
- **Pinning**: `P` saves the selected clip into a built-in Pinned folder, and folder items are exempt from cleanup.
- **Export and import**: Export your folders to a single readable JSON file and import it on another machine. Folders merge by name, and importing the same file twice changes nothing.
- **Type and source filters**: Narrow the list by content type (text, image, file) or by the application a clip came from.
- **Jump to the owning folder**: A card shows which folder it is saved in, and clicking that jumps there.
- **Custom titles**: Name a clip from the right-click menu, and find it later by that name.
- **`Ctrl+1` to `Ctrl+9`**: Paste the nth clip from the top of the list.
- **Typography**: Choose the interface and clip fonts, their weights, and the overall text size.

### Changed
- **Smoother window animation**: The slide now eases out, and its frames are paced against a fixed clock. It previously moved linearly and slept for an interval Windows rounded up unevenly, which read as stutter.
- Retention is only evaluated when a new clip arrives, not when an existing one is pasted back.

### Fixed
- **Image transparency is preserved**: Images were written through the WebView, which could only produce an opaque bitmap, so a transparent PNG pasted with a black background. Images are now written as CF_DIBV5, CF_DIB and PNG.
- **History retention and the item limit did nothing**, so history grew without bound on disk and in the database.
- **The `P` shortcut did nothing**, although the README and the in-app documentation both listed it.
- **File clips** now list their file names instead of printing every full path, and report a file count rather than a character count.

### 新增
- **文件**：复制文件或文件夹会记录其路径，粘贴时重新写回剪贴板，资源管理器可以直接“粘贴”。
- **富文本**：格式、链接与表格以 HTML 与 RTF 保存，粘进文字处理软件时保留格式，不再被压成纯文本。
- **历史保留**：可设定保留多少天、多少条；保存到文件夹中的内容不受这两个限制。该设置此前存在但从未生效。
- **收藏**：按 `P` 将选中的剪贴存入内置“收藏”文件夹，文件夹内的内容豁免清理。
- **导出与导入**：将文件夹导出为一个可读的 JSON 文件，在另一台设备导入。同名文件夹会合并，同一份文件导入两次不会产生任何变化。
- **类型与来源筛选**：按内容类型（文本 / 图片 / 文件）或来源应用缩小列表。
- **跳转到所在列表**：卡片会显示它保存在哪个文件夹，点击即可跳转过去。
- **自定义标题**：在右键菜单给剪贴起名，之后可按该名字搜到它。
- **`Ctrl+1` 至 `Ctrl+9`**：直接粘贴列表前几条。
- **字体**：可分别设置界面与内容的字体、粗细，以及整体字号。

### 修改
- **窗口动画更顺滑**：滑动改为缓出曲线，且每帧按固定时钟对齐。此前为线性位移，加上一个会被 Windows 不均匀向上取整的 sleep，表现为抖动。
- 保留策略只在**有新剪贴进入**时评估，粘贴回已有条目时不再评估。

### 修复
- **图片透明通道得以保留**：此前图片由前端 WebView 写入，只能产出不透明位图，导致透明 PNG 粘出后是黑底。现改为写入 CF_DIBV5、CF_DIB 与 PNG。
- **历史保留与条数上限完全没有生效**，导致磁盘与数据库中的历史无限增长。
- **`P` 快捷键完全无效**，尽管 README 与应用内文档都列出了它。
- **文件剪贴**现在列出文件名，不再打印每条完整路径；页脚显示文件数量而非字符数。

## v1.5.0

### Added
- **Multi-size Display Layout**: Introduced customizable UI sizing options ("Default" and "Compact") for better use of screen real estate on smaller displays. The window resizes instantly upon selection.
- **Background Update Check**: Added a non-intrusive UI indicator for new updates in the main window and a direct "Update" button in the Settings footer (#15).

### Changed
- Refined the default clipboard card aspect ratio to be more perfectly square for improved aesthetics.
- Optimized text contrast in Dark Mode to ensure the app name header remains visible on vibrant colored backgrounds.
- Unified toolbar icon colors (Add, Update, Settings) to adapt seamlessly to the system theme.

### Fixed
- Fixed sub-pixel rendering gaps and black borders around selected cards in specific display scales.
- Restored robust Mica effect persistence across window toggling, system sleep, and wake cycles (#9, #10).
- Fixed redundant UI state updates causing visual tearing when deleting items via context menus (#14).
- Added reliable image cleanup from the filesystem before cascaded folder deletions in SQLite (#11, #14).

### 新增
- **多尺寸界面布局**：加入可自定义的“界面大小”选项（默认 / 紧凑），紧凑模式专为小屏设备优化，切换时主窗口即时丝滑缩放。
- **后台更新检测**：增加后台静默更新检测机制，在主界面显示无打扰更新图标，并在设置页底部提供直接更新按钮 (#15)。

### 修改
- 优化默认剪贴板卡片的宽高比例，使其视觉上更接近完美的正方形，提升整体美感。
- 增强了深色模式下的文字对比度，确保卡片来源应用的标题在鲜艳背景上依旧清晰可读。
- 统一主界面工具栏图标（添加、更新、设置）颜色，使其与系统主题完美适配。

### 修复
- 修复了在部分缩放比例下，选中卡片时边缘由于亚像素渲染（Sub-pixel rendering）导致的黑色缝隙问题。
- 修复了窗口频繁呼出、系统睡眠唤醒后 Mica 材质特效丢失的问题 (#9, #10)。
- 修复了通过右键菜单删除项目时，冗余的状态更新导致的视觉闪烁问题 (#14)。
- 完善了 SQLite 级联删除机制，彻底解决了删除目录前残留实体图片文件的问题 (#11, #14)。



## v1.3.8

### Added
- Configurable Auto-Paste shortcut method (Shift + Insert / Ctrl + V) with extended virtual key event simulation (#13)
- Confirmation dialog when deleting folders, with cascading deletion of folder clips and associated images (#14)
- Option to preserve saved folder items when clearing clipboard history (#11)

### Fixed
- Fixed multi-monitor and mixed-DPI display coordinate scaling issues (#9)
- Fixed backdrop material (Mica / Mica Alt / Clear) and dark theme persistence across system sleep, wake, and lock screen (#10)
- Fixed default selection and scroll reset to the first clip upon window reopen or receiving new clips (#12)
- Fixed clip list falling into an empty state when deleting the currently selected folder by automatically switching back to All Clips (#14)

### 新增
- 支持在设置中选择自动粘贴快捷键模式（Shift + Insert / Ctrl + V），并增强虚拟按键事件模拟与终端兼容性 (#13)
- 删除文件夹时增加确认对话框，并支持级联删除文件夹内的历史项及磁盘关联图片 (#14)
- 清空剪贴板历史时默认保留已保存至文件夹中的项目 (#11)

### 修复
- 修复多显示器及不同 DPI 缩放下的窗口定位与尺寸错位问题 (#9)
- 修复在系统休眠、锁屏唤醒后 Mica/Mica Alt/Clear 材质与深色主题属性失效的问题 (#10)
- 修复重新唤出窗口或接收新剪贴板内容时未自动定位并选中第一项的问题 (#12)
- 修复删除当前选中的文件夹后未自动回退至"全部历史"导致列表显示为空的问题 (#14)

## v1.3.7

### Added
- German, French, and Japanese language support

### Improved
- Winget release pipeline: hash verification step added before publishing to winget-pkgs to prevent stale-hash mismatches; release tag now explicitly pinned

### 新增
- 新增德语、法语、日语语言支持

### 优化
- Winget 发布流程：在发布至 winget-pkgs 前增加哈希值校验步骤，防止哈希不匹配问题；发布时明确指定 release tag

## v1.3.6

### Added
- Support floating window above the taskbar (toggle in Settings)
- Every release is now automatically scanned with VirusTotal (70+ antivirus engines) — scan results are linked in the release notes

### 新增
- 窗口支持浮动在任务栏上层（可在设置中开启/关闭）
- 每次发布版本现在会自动通过 VirusTotal（70+ 款杀毒引擎）进行安全扫描，扫描结果链接附在 Release 说明中

## v1.3.5

### Added
- Native rounded corners support for all window effects (Mica, Mica Alt, Clear) using Windows 11 DWM — toggle on/off in Settings

### Fixed
- Fixed TypeScript build error caused by missing Vite client types (`import.meta.env`)

### 新增
- 所有窗口效果（Mica、Mica Alt、Clear）均支持原生圆角，通过 Windows 11 DWM 实现，可在设置中开启/关闭

### 修复
- 修复因缺少 Vite 客户端类型导致的 TypeScript 构建错误（`import.meta.env`）

## v1.3.4

### Added
- Brand new native style look with Windows Mica and Mica-Alt window effects for a seamless, beautiful appearance that blends with your desktop

### 新增
- 全新原生风格外观，支持 Windows Mica 和 Mica-Alt 窗口效果，与桌面完美融合，带来更精美的视觉体验

## v1.3.3

### Changed
- Refined UI layout: reduced window height, tightened card spacing, fixed control bar height, and removed CSS shadow in Clear window effect mode

### 变更
- 优化界面布局：减小窗口高度、收紧卡片间距、固定控制栏高度，并在"无效果"窗口模式下移除 CSS 阴影

## v1.3.2

### Fixed
- Fixed hotkey toggle broken after changing hotkey in settings (issue #6)
- Fixed winget package missing arm64 installer by switching to NSIS setup.exe for architecture detection (issue #7)

## v1.3.1

### Fixed
- Removed white/alpha border around settings window in dark mode

