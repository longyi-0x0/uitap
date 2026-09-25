import Foundation

let usage = """
uitap — macOS 桌面观测与输入合成，输出均为单行 JSON

  doctor                                    授权与依赖自检
  screens                                   显示器列表（点坐标 bounds / 像素 / scale）
  windows   [--all] [--app N] [--title N] [--layer N] [--minWidth N] [--minHeight N]
            [--frontOnly] [--limit N]
  frontmost                                 当前前台应用

  shot      [--window ID | --region X,Y,W,H | --display N] [--path P] [--maxPx N]
  crop      --in P [--out P] [--region X,Y,W,H | --regionPoints X,Y,W,H] [--maxPx N]
  pixel     --path P --at X,Y [--at X,Y ...] [--units pixel|point]
  diff      --before P --after P [--region X,Y,W,H] [--threshold N] [--minPixels N]
            [--maxRegions N] [--units pixel|point]

  wait-stable [--window ID | --region X,Y,W,H | --display N] [--interval MS] [--timeout MS]
            [--threshold R] [--stableSamples N]

  click     --at X,Y [--button left|right|middle] [--count N] [--hold MS]
  move      --to X,Y
  drag      --from X,Y --to X,Y [--button N] [--duration MS] [--steps N]
  scroll    [--at X,Y] [--dx N] [--dy N]
  type      --text S [--delay MS]
  key       --combo "cmd+shift+t" [--repeat N]
  activate  [--app NAME | --pid N | --window ID] [--settle MS]

  tap       --at X,Y [同 wait-stable 的参数] [--button N] [--count N] [--keep]

坐标默认是全局点坐标，左上角为原点，与 CGEvent 光标坐标一致。
shot 会在 PNG 旁写同名 .json 记录 origin 与 scale，其后 pixel / diff 无需再指定。
"""

let argv = Array(CommandLine.arguments.dropFirst())
guard let command = argv.first else { fail("missing command; run `uitap help`") }
let args = Args(Array(argv.dropFirst()))

switch command {
case "help", "--help", "-h":
    print(usage)
    exit(0)
case "doctor": cmdDoctor(args)
case "screens": cmdScreens(args)
case "windows": cmdWindows(args)
case "frontmost": cmdFrontmost(args)
case "shot": cmdShot(args)
case "crop": cmdCrop(args)
case "pixel": cmdPixel(args)
case "diff": cmdDiff(args)
case "wait-stable": cmdWaitStable(args)
case "click": cmdClick(args)
case "move": cmdMove(args)
case "drag": cmdDrag(args)
case "scroll": cmdScroll(args)
case "type": cmdType(args)
case "key": cmdKey(args)
case "activate": cmdActivate(args)
case "tap": cmdTap(args)
default:
    fail("unknown command: \(command)")
}
