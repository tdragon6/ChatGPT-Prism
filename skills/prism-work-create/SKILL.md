---
name: prism-work-create
description: 为 ChatGPT Prism 创作或修改可导入的启动动画，适用于作品封面、配色、参数与播放行为
---

# Prism 启动动画

## 作品目录

交付以作品 ID 命名的目录，内含 `meta.json`、HTML 入口和播放所需的本地资源。ID 长度为 2–64 位，以小写英文字母开头，可含小写字母、数字和单个分隔连字符。路径使用 `/`，文件名使用英文字母、数字、点、下划线或连字符，避免隐藏文件、符号链接和大小写重名。最多 100 个文件，单文件最多 8 MiB，总计最多 32 MiB。

`meta.json` 必填 `apiVersion`（`1`）、`type`（`"startup"`）、`id`、`name`、`version`（数字 `x.y.z`）、`themes` 和 `entry`。写明作者 `author` 和简短说明 `description`，文案末尾省略句号。为作品提供 16:9 画面截图，通过 `preview` 指定 PNG、WebP 或 JPEG 封面。

随安装包分发的作品以独立文件提供，不把作品内容或 ID 写入宿主程序。通过作品 `meta.json` 的 `default: true` 指定首次安装的首选作品，每个安装包最多指定一个；作品更换 ID 时，在新作品的 `legacyIds` 数组中列全支持迁移的历史 ID，不可重复归属或形成循环。宿主保留同版本或更高版本的本地作品；随包版本更高时，先归档旧副本再更新。更名时迁移选中项及参数，并将旧目录归档到应用数据目录的 `startup-archive/`。

`themes` 声明支持的黑曜 `dark` 和极昼 `light`，`languages` 声明支持的中文 `zh` 和英文 `en`。按 `context.theme` 切换配色，按 `context.locale` 切换界面文案。需要独立入口时用 `locales`，用 `i18n.<语言>.name`、`description` 和 `parameters.<键>.label` 翻译作品和控件文字。

作品 ID 和目录名应与作品的英文名称对应，例如「终端序章 / Terminal Prelude」使用 `terminal-prelude`。

作品 README 可选，按需提供单语或双语说明。中文使用 `README.md`，英文使用 `README_EN.md`；提供双语时必须分别存放，每个 README 只使用对应语言，不在同一文件中混排中英文。若提供 README，列出实际支持的语言、主题、参数默认值、范围及播放方式。

入口使用 UTF-8 完整 HTML，包含 `head` 和 `body`。Prism 会将资源内联后放在沙箱中播放。用 HTML 的 `src`、`poster`、本地样式表链接或 CSS 的 `url()` 引用资源。用经典脚本，样式表由 HTML 加载，媒体从已有的 `audio` 或 `video` 元素播放。沙箱禁用联网请求和 CSS `@import`，JavaScript 动态拼接的资源路径不会内联。

## 作品许可证与第三方许可证

- 新建和修改的作品，其自有内容统一采用 MIT 许可：在 `meta.json` 中设置 `"license": "MIT"`，并在作品根目录提供完整的作品许可证 `LICENSE`，填写作品权利人的署名；如提供 README，在相应语言的 README 中注明 MIT。
- 引用第三方开源代码或素材时，统一称其许可文件为「第三方许可证 / Third-party license」，单独保存在本作品的 `THIRD-PARTY-LICENSES/<项目名>-LICENSE`。该目录专用于第三方许可证，与作品根目录的 `LICENSE` 区分；未引用第三方内容时无需创建空目录。完整保留原作者版权署名和许可文本，原项目要求随附的 NOTICE 等声明也一并保留。不要把第三方署名混入作品许可证，也不要将第三方原有许可改写为 MIT。
- 在作品的 `THIRD-PARTY-NOTICES.md` 中列明引用项目、来源链接、所用内容、第三方许可证名称及对应文件；说明中的链接标为「第三方许可证 / Third-party license」。已知版本或提交时一并记录。创作前确认引用内容允许按本项目的 MIT 作品方式分发，无法满足原授权条件时更换素材。
- 交付、导入和打包时保留上述文件；发布市场索引时将许可证和声明纳入 `files`，同步字节数、SHA-256 和总摘要。

## 画面与控件

画面适配窗口尺寸，外沿保持无边框。开场、状态变化和收束要清晰，窄窗口下仍能读清关键文字，完整一轮在最慢速度下也应于 45 秒内结束。

右上角把 Audio 和 Skip 放在同一行，用 `align-items: center` 对齐。以宿主 Skip 的 `top: 5.6%`、`right: 3.5%` 为锚点，控件行高 `12px`、上下内边距 `5px`。当 `context.hostControls?.skip` 为真，用 `visibility: hidden` 隐藏作品的 Skip 并保留占位。预览和独立播放显示作品自己的 Skip。为按钮提供可读的无障碍名称和明确的键盘焦点样式。

## 可调参数

`parameters` 最多 24 项。每项需要唯一 `key`、`label`、`type` 和有效的 `default`。键名以小写字母开头，可含英文字母、数字和下划线。支持以下类型。

- `boolean` 用布尔值
- `number` 提供 `min`、`max`、`step`
- `text` 提供 `maxLength`
- `color` 用 `#RRGGBB`
- `select` 提供含 `value` 和 `label` 的 `options`

`speed` 默认 `1`，使用正数范围。若提供音乐，用布尔型 `sound` 和 `0–1` 的 `volume`，音量默认 `0.5`；宿主以 `meta.json` 中声明的参数默认值为准。若提供身份名称，`identity` 使用 `pattern: "identity"` 和不超过 32 的 `maxLength`。从 `context.parameters` 读取并应用每项设置，独立播放沿用同样的默认值。

## 播放与检查

- 从 `window.__PRISM_CONTEXT__` 读取初始配置。内嵌播放且尚无初始配置时，先注册监听，再发送 `parent.postMessage({ type: "prism:ready" }, "*")`，收到 `prism:init` 后启动。两种方式同时出现时只启动一次。仅接收来自 `parent` 的宿主消息。
- 收到 `prism:update` 时更新配色，并将 `context.parameters.sound` 和 `volume` 的变化即时应用于音乐与播报，保持播放进度；调整音量不应重新创建音频上下文或重新开始动画。根据 `context.reducedMotion` 或 `prefers-reduced-motion` 减少大幅移动与闪动。
- 自然结束时停止动画和音频，向宿主发送 `parent.postMessage({ type: "prism:done" }, "*")`。跳过时附加 `reason: "skip"`。Prism 在预览中循环播放，启动时播放一轮。收到 `prism:dispose` 或页面卸载时清理音频、帧请求、计时器和监听器。
- 在 Prism 导入作品，检查每个声明的语言和主题。以窄窗和宽窗确认文字可读、Audio 和 Skip 在同一水平线上，并检查参数、主题切换、循环、跳过与关闭后静音。

发布到社区市场时，使索引的 `manifest` 与 `meta.json` 一致。`sourceUrl` 可填写相对于索引所在目录的作品目录，也可填写跨仓库、跨站点的完整 HTTP/HTTPS 目录地址，以 `/` 结尾；建议使用 HTTPS。完整地址不包含凭据、查询参数或片段。`files` 仍记录作品内各文件的相对路径、字节数和 SHA-256，下载地址由 `sourceUrl` 与文件相对路径组合；条目的 `size` 为总字节数，`sha256` 为排序后紧凑 JSON 清单的摘要。清单对象按 `path`、`size`、`sha256` 写入，路径按 ASCII 不区分大小写排序，`_`、`-`、`.`、`/` 依次排在字母数字前。
