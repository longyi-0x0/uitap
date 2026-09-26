# uitap

桌面观测与输入合成。给模型一组可直接调用的工具：列窗口、截图、取色、按色找像素、比图、点按拖拽、键入组合键。

单个二进制同时是命令行与 MCP server，没有 Node 或 Python 运行时依赖。

| crate | 职责 |
| --- | --- |
| `uitap-core` | 平台无关内核：几何与锚点换算、位图与差分、后端 trait。无任何平台调用，可在任意目标上编译与测试 |
| `uitap-macos` | macOS 后端：窗口列表与输入合成走 CoreGraphics，截图走 `screencapture` |
| `uitap-platform` | 按目标平台选择后端，暴露平台专属能力（组合键解析、主线程服务） |
| `uitap-ops` | 操作层：编排后端调用并产出 JSON 契约。CLI 与 MCP 共用，契约只定义一次 |
| `uitap-cli` | 命令行入口，`bin uitap` |
| `uitap-mcp` | MCP server（`rmcp`），由 `uitap mcp` 启动 |

`legacy-swift/` 是此前的 Swift 实现，作为输出对拍基准保留。

## 构建

```bash
tools/uitap/build.sh
```

编译出 `target/release/uitap`，并在 `bin/uitap` 建指向它的软链。`.vscode/mcp.json` 引用 `bin/uitap mcp`，切换实现时不必改配置。跑完会打印一次 `doctor` 结果。

需要 Rust 1.88 以上（`rmcp` 的要求）。

## 测试

```bash
cargo test                          # 87 项单测：几何换算、JSON 数值约定、差分区域、颜色命中与聚簇、坐标口径、应用匹配、主线程派发
python3 parity.py                   # 与 Swift 版逐字节对拍
```

`parity.py` 自己生成测试图，只依赖标准库。它对同一组命令跑两个二进制并比对归一化后的 JSON，动态字段（文件路径、时间戳）除外。

对拍需要先编译 `legacy-swift/` 里的 Swift 版作为基准：

```bash
swift build -c release --package-path legacy-swift
python3 parity.py
```

不编译也可以，`parity.py` 会指明缺哪个二进制。

## 元素层的边界

**`AXEnabled` 不是所有角色都有。** `ui_find --enabled` 只保留该属性明确为真的元素，
像 `AXTextArea` 这类控件常常不带这个属性，会被一起滤掉。查文本区域时不要加 `--enabled`。

**路径下标会随树变化漂移。** 展开一个菜单、打开一个面板都会改变索引链。跨多次操作复用旧
`path` 之前，先确认树没变。

**自绘界面查不到元素。** 元素树来自应用自己暴露的辅助功能信息。画布类应用（部分 Flutter、
游戏引擎、自定义渲染）通常只暴露一个大容器，这时只能走坐标路径。

**遍历代价与树的大小成正比。** 每次读属性都是一次进程间调用，节点多时一次全量遍历约需 1 秒
（Safari 约 200 个节点即在此量级）。`ui_find` / `ui_wait_for` 默认按 `maxNodes: 200` 遍历，
限制 `depth` 与小 `maxNodes` 能显著减少耗时；`ui_wait_for` 每轮都重新遍历，超时前的轮询次数
按这个量级估算。

## 多 agent 并发

多个 agent 同时驱动同一套鼠标键盘会互相破坏：A 正在点击并验证结果，B 的点击插进来，A 的
验证结论就是错的。因此输入操作会先取一次互斥租约。

**只锁输入。** 读操作（截图、取色、比图、读元素树）不改变状态，即便与别人的输入交错，
最坏也只是「看到对方的动作」。真正会互相破坏的是两处同时往同一个界面里送输入。

**`ui_tap` 整段独占。** 它必须覆盖「点击 → 等稳定 → 比对」全程，否则验证结论会被污染。
只在点击那一瞬独占是不够的。

**拿不到不会干等。** 返回的是可执行的指引，而不是超时或静默交错：

```
ui_click 未执行：另一个 agent（pid:30104）正在执行 ui_tap。建议 1000ms 后重试，
或加 waitMs 让它自己等。对方最长持有可能还有 29s（崩溃时的兜底时限）。
若确认对方已卡死，用 ui_lease(action="clear") 清除。
```

「建议重试间隔」与「持有上限剩余」是两个不同的量：前者是轮询量级（1 秒），后者是持有者
崩溃时的兜底时限（默认 30 秒）。只报后者会让人以为要等 30 秒而放弃。

| 参数 / 工具 | 作用 |
| --- | --- |
| `waitMs` | 被占用时最多等多久，默认 5000；设为 0 则立即返回忙碌 |
| `noLock` | 跳过互斥检查。仅在确认无并发时用 |
| `ui_lease(action="status")` | 查看当前持有者、在做什么、剩余时限 |
| `ui_lease(action="clear")` | 强制清除。持有者卡死又没到 TTL 时的逃生口 |

命令行对应 `--wait MS` / `--noLock` / `uitap lock [--action status|clear]`。

**识别是谁在占用。** 默认标识是 `pid:<进程号>`。多 agent 场景建议各自设 `UITAP_AGENT`
环境变量，占用信息里就能直接看出是谁。同一个 agent 的并发操作会被标成「同一个 agent 的
另一次操作」，便于区分「自己挡了自己」与「别人在忙」。

**三条不变量**（在 `crates/uitap-core/src/lease.rs` 的测试里逐条覆盖）：

1. 不会死锁 —— 租约带到期时间；持有者进程消失（用信号 0 探测）或超时都可被接管。
2. 不会误删 —— 释放前比对 token，被接管的原持有者释放时是空操作。
3. 接管是排他的 —— 过期文件先 `rename` 到唯一名字再删，`rename` 只可能有一个进程成功。

## 两个平台约束

写在实现里，改动相关代码前先读这两条。

**激活必须在主线程。** `NSRunningApplication` 的激活只有在本进程主线程上执行才会落地：从工作线程发起时，系统会搁置该请求，无论怎样轮询都观察不到结果（实测主线程 3/3 成功、工作线程 2/3 超时）。因此 `uitap mcp` 把主线程留给 AppKit，事件循环由 `uitap-macos::mainthread` 驱动；server 本身跑在独立线程上。`MacBackend::activate` 会自动把调用投递到主线程，已经是主线程（CLI 场景）或服务未启动时则原地执行。

**`screencapture` 拒绝写以点开头的文件名。** 它在 stderr 报错但退出码仍为 0，因此截图后必须核实产物存在，不能只看退出码。同理，等待稳定的中间帧文件名不能带点前缀。

## 应用匹配

`activate --app` 依次尝试：名字或 bundle id 末段完全相等，名字包含查询词，bundle 末段包含查询词。bundle id 只比末段，这样 `Finder` 能对上 `com.apple.finder`，而不会误命中 `com.apple.SafariPlatformSupport.Helper` 这类把目标词放在中间段的辅助进程。无法成为前台应用的进程（激活策略为 prohibited）不参与匹配。

`frontmost` 字段是激活后**确认过的**结果。若轮询窗口内系统仍未切换，会返回 `false` —— 此时请求已发出，再调一次通常即可成功。

## 授权

两项系统授权，缺一项工具就废一半。在 `doctor` 里查：

| 授权 | 系统设置位置 | 缺了会怎样 |
| --- | --- | --- |
| 屏幕录制 | 隐私与安全性 → 屏幕录制 | 截图报错；`shot` 返回 `screencapture failed` |
| 辅助功能 | 隐私与安全性 → 辅助功能 | 点击、拖拽、键入全部静默失效 |

授权对象是**承载 MCP server 的进程**。VS Code 里跑就勾 VS Code；在终端里手工跑就勾那个终端程序。改动授权后需要重启对应进程。

## 坐标

对外一律是**全局点坐标**，左上角为原点，与 `CGEvent` 光标坐标同一套。多显示器时第二块屏的 x 从主屏宽度起算，可以为负。

截图带锚点：`shot` 会在 PNG 旁写同名 `.json`，记下 `origin`（该图左上角对应的全局点）与 `scale`（像素宽 ÷ 点宽）。因此：

- 图上像素 `p` 对应的全局点是 `origin + p / scale`。
- 传 `--units point` 时 `pixel` / `diff` 的入参按点坐标解释，内部换算成像素。
- `scale` 由最终图像文件反推，所以缩放过的图换算依然成立。

**图旁没有这份 `.json` 时**（用户给的外部截图、别处拷来的图），该图没有「点」可言：
`region` 与点坐标一律按**图像像素**解释，`zoom` / `pixel` / `diff` / `find-pixels` 的返回里
`units` 字段说明这次用的是哪一种。显式要了点坐标（`--units point`）却没有锚点时才报错，
错误里给出图名与该改怎么传。

`region` 越出这张图时，报错给出越出的是哪几条边与这张图在该坐标系下的范围，例如：

```
region 超出这张图：x 1180..1440 越出 1920..3968。这张图是 1500x791 点（origin 1920,0 / scale 0.73），region 按点坐标给
```

Retina 上 `scale` 为 2；非 Retina 与缩放显示器为 1。

## 两种截图的差别

| 方式 | 取到什么 |
| --- | --- |
| `--window ID` | 窗口自身内容，不受遮挡影响，不含阴影 |
| `--region X,Y,W,H` | 屏幕合成结果，含遮挡与上层窗口 |
| 都不给 | 主显示器全屏 |

两者在窗口边缘的反锯齿像素上会有 1px 级差异。验证「用户看到的画面」用 `--region`；验证「窗口自己的状态」用 `--window`。

## 窗口排序

`windows` 按面积降序；面积相同时按前后叠放顺序，最前的在前，`z` 字段给出该序号（0 为最前）。同应用多开同尺寸窗口时靠 `z` 区分，不要用 id 排序。

## 输出约定

所有输出是单行 JSON。整数不带小数点，比例固定 4 位小数并去尾随零，空字段被剔除，键按字典序。临时截图落在 `/tmp/uitap/`，超过 240 个文件时自动裁掉较旧的一半。

## 工具

两组工具走的是两条不同的路径，可以混用。

**观测与输入**（23 个中的 16 个）走屏幕像素与合成事件：能用在不暴露辅助功能信息的界面上，
代价是必须抢前台、必须对准坐标。

| 工具 | 作用 | 备注 |
| --- | --- | --- |
| `ui_doctor` | 授权与依赖自检 | 任何工具报权限错误时先看这个 |
| `ui_screens` | 显示器列表 | `bounds` 是点坐标，`pixels` 是像素，`scale` 是倍率 |
| `ui_windows` | 窗口列表 | 按面积降序，同尺寸按前后叠放（`z` 越小越前），默认 25 个 |
| `ui_shot` | 截图 | 默认只回锚点与短 id；`includeImage` 才返回图像 |
| `ui_zoom` | 局部放大 | 按点坐标裁剪并缩放，用于辨认细节 |
| `ui_pixel` | 取点颜色 | 传 `expect` 时直接返回 `match` 布尔值，省掉模型侧的比较 |
| `ui_find_pixels` | 按颜色找像素 | 回命中数、包围盒与连通聚簇（位置与像素数）；代替「截图之后自己写循环扫像素」 |
| `ui_diff` | 两张截图比对 | 返回变化区域，验证界面是否响应首选这个 |
| `ui_wait_stable` | 等画面不动 | 动画、加载结束后再观察 |
| `ui_click` | 点击 | `count: 2` 为双击 |
| `ui_drag` | 拖拽 | 默认 300ms 分 20 段，太快会被应用丢帧 |
| `ui_scroll` | 滚轮 | `dy` 为正向上 |
| `ui_type` | 键入文本 | 支持中文；应用丢字时调大 `delayMs` |
| `ui_key` | 组合键 | 如 `cmd+shift+t`；可用 `esc`、`return`、`f5` 这类键名 |
| `ui_activate` | 切前台 | `frontmost` 是确认过的结果，不是发出请求就算成功 |
| `ui_tap` | 点击 → 等稳定 → 比对 | 一次调用替代 `ui_shot` + `ui_wait_stable` + `ui_diff` 三步；整段独占互斥租约 |

除 `ui_doctor`、`ui_screens`、`ui_windows`、`ui_shot`、`ui_zoom`、`ui_pixel`、`ui_find_pixels`、
`ui_diff`、
`ui_wait_stable` 这几个只读工具外，其余输入类工具（`ui_click`、`ui_drag`、`ui_scroll`、
`ui_type`、`ui_key`、`ui_activate`、`ui_tap`）动手前会先取一次**跨进程互斥租约**，
多个 agent 共用一个桌面时不会互相踩。见「多 agent 并发」。

**辅助功能元素**（6 个）走 macOS 的 AX 接口：按语义定位元素，**不移动鼠标、不切换前台应用**，
也不受窗口遮挡影响。代价是依赖目标应用暴露可用的元素树，自绘界面（部分 Flutter、游戏、
Qt 自定义绘制）往往查不到东西。

| 工具 | 作用 | 备注 |
| --- | --- | --- |
| `ui_tree` | 读元素树 | 拍平成列表，广度优先；`depth` 默认 12、`maxNodes` 默认 200，超出时返回 `truncated` |
| `ui_find` | 按条件查元素 | 条件（`role`/`subrole`/`title`/`value`/`identifier`）按「包含」匹配、大小写不敏感，给出的每一项都要满足 |
| `ui_actions` | 列出元素可用动作 | 返回空数组表示该元素不可交互 |
| `ui_press` | 对元素执行动作 | 默认 `press`（相当于点击）；失败时会列出该元素实际支持的动作 |
| `ui_set_value` | 设置元素的值 | 可直接写入文本框，不触发键盘输入 |
| `ui_wait_for` | 等元素出现 | 比反复截图比对省 token；空条件会被拒绝 |

### 元素路径

`ui_tree` 与 `ui_find` 返回的 `path` 是从应用根元素开始、逐层走子元素的索引链，如 `0.1.3`。
它无状态：每次操作都从根重新走一遍解析，因此不需要跨调用保存句柄。树结构变了旧路径就失效，
重新取一次即可。空路径表示应用根元素本身，它不能作为操作目标。

### 什么时候用哪条路

优先试元素路径：定位准、不受遮挡影响、不打断用户正在做的事。查到空树或元素没有可用动作时，
退回坐标路径。两者可以衔接——`ui_find` 返回的 `bounds` 就是屏幕点坐标，可直接喂给 `ui_click`。

## 命令行

`uitap help` 列出全部子命令。举几个：

```bash
uitap tree --app Finder --depth 3 --maxNodes 40
uitap find --app Finder --role AXMenuBarItem --title 文件
uitap press --app Finder --path 0.2 --action press
uitap wait-for --app Finder --role AXDialog --timeout 5000

uitap windows --app Code --layer 0 --minWidth 800
uitap shot --window 101664 --path /tmp/w.png
uitap pixel --path /tmp/w.png --at 400,300 --units point
uitap find-pixels --path /tmp/w.png --color "#2F6BFF" --region 3600,600,400,300
uitap diff --before a.png --after b.png --units point
uitap tap --at 15,15 --region 0,0,700,700
uitap click --at 400,300 --count 2
uitap key --combo "cmd+shift+t"
uitap type --text "你好"
```

`wait-stable` 的 `--threshold` 是变化比例上限（默认 0.0006）；`diff` / `tap` 的 `--threshold` 是单像素色差阈值（默认 24）；`find-pixels` 的 `--tolerance` 是单通道容差（默认 12），`--color` 可给多次，命中其中任一个即算。

## 平台

当前实现 macOS。`crates/uitap-core` 与命令实现不含平台分支，其他平台接入时只需补齐一个 `Backend` 实现；未实现的平台会明确返回 `unsupported`，而不是空结果。

`UITAP_BIN` 可覆盖 MCP server 使用的二进制路径，默认取 `bin/uitap`。
