# uitap

桌面观测与输入合成。给模型一组可直接调用的工具：列窗口、截图、取色、比图、点按拖拽、键入组合键。

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
cargo test                          # 37 项单测：几何换算、JSON 数值约定、差分区域、应用匹配、主线程派发
python3 parity.py                   # 与 Swift 版逐字节对拍
```

`parity.py` 自己生成测试图，只依赖标准库。它对同一组命令跑两个二进制并比对归一化后的 JSON，动态字段（文件路径、时间戳）除外。

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

`ui_doctor`、`ui_screens`、`ui_windows`、`ui_shot`、`ui_zoom`、`ui_pixel`、`ui_diff`、`ui_wait_stable`、`ui_click`、`ui_drag`、`ui_scroll`、`ui_type`、`ui_key`、`ui_activate`、`ui_tap`。

`ui_tap` 是「点击 → 等稳定 → 与点击前比对」的合成动作，一次调用返回变化的点坐标，替代 `ui_shot` + `ui_wait_stable` + `ui_diff` 三次往返。

`ui_shot` 默认只返回锚点与短 id（`s1`、`s2`…），不带图像。要看图才传 `includeImage`，此时返回缩放后的副本，原图不受影响。后续工具可以直接用 id 替代路径。

## 命令行

`uitap help` 列出全部子命令。举几个：

```bash
uitap windows --app Code --layer 0 --minWidth 800
uitap shot --window 101664 --path /tmp/w.png
uitap pixel --path /tmp/w.png --at 400,300 --units point
uitap diff --before a.png --after b.png --units point
uitap tap --at 15,15 --region 0,0,700,700
uitap click --at 400,300 --count 2
uitap key --combo "cmd+shift+t"
uitap type --text "你好"
```

`wait-stable` 的 `--threshold` 是变化比例上限（默认 0.0006）；`diff` / `tap` 的 `--threshold` 是单像素色差阈值（默认 24）。

## 平台

当前实现 macOS。`crates/uitap-core` 与命令实现不含平台分支，其他平台接入时只需补齐一个 `Backend` 实现；未实现的平台会明确返回 `unsupported`，而不是空结果。

`UITAP_BIN` 可覆盖 MCP server 使用的二进制路径，默认取 `bin/uitap`。
