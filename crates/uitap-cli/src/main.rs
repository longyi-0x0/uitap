//! uitap 命令行入口。所有输出是单行 JSON，坐标默认是全局点坐标（左上原点）。
//!
//! 这一层只做三件事：解析参数、调用操作层、打印结果。`mcp` 子命令起 MCP server。

mod args;
mod mapping;
mod out;

use args::Args;
use out::{emit, fail, finish};
use uitap_ops::{image, input, observe, tap as tap_op, wait, OpResult};
use uitap_platform::{capture_available, open, parse_combo};

const USAGE: &str = r#"uitap — 跨平台桌面观测与输入合成，输出均为单行 JSON

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

  click     --at X,Y [--button left|right|middle] [--count N]
  move      --to X,Y
  drag      --from X,Y --to X,Y [--button N] [--duration MS]
  scroll    [--at X,Y] [--dx N] [--dy N]
  type      --text S [--delay MS]
  key       --combo "cmd+shift+t" [--repeat N]
  activate  [--app NAME | --pid N | --window ID]

  tap       --at X,Y [--window ID | --region X,Y,W,H | --display N] [--timeout MS]
            [--stableSamples N] [--button N] [--count N] [--keep]

  mcp                                       以 stdio 起 MCP server

坐标默认是全局点坐标，左上角为原点。shot 会在 PNG 旁写同名 .json 记录 origin 与 scale，
其后 pixel / diff 无需再指定。wait-stable 的 --threshold 是变化比例上限（默认 0.0006），
diff 的 --threshold 是单像素色差阈值（默认 24）。
"#;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first().cloned() else {
        fail("missing command; run `uitap help`");
    };
    let args = Args::new(&argv[1..]);

    match command.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            std::process::exit(0);
        }
        "mcp" => run_mcp(),
        // 纯图像处理与平台无关，不需要后端。
        "crop" => dispatch_crop(&args),
        "pixel" => dispatch_pixel(&args),
        "diff" => dispatch_diff(&args),
        _ => {
            let backend = open();
            dispatch_platform(&command, &args, &backend);
        }
    }
}

fn require<'a>(a: &'a Args, key: &str) -> &'a str {
    match a.str(key) {
        Some(value) => value,
        None => fail(format!("--{key} is required")),
    }
}

fn dispatch_crop(a: &Args) -> ! {
    let input = std::path::PathBuf::from(require(a, "in"));
    finish(image::crop(&mapping::crop_request(a, input)))
}

fn dispatch_pixel(a: &Args) -> ! {
    let path = std::path::PathBuf::from(require(a, "path"));
    finish(image::pixel(&mapping::pixel_request(a, path)))
}

fn dispatch_diff(a: &Args) -> ! {
    if a.str("before").is_none() || a.str("after").is_none() {
        fail("--before and --after are required");
    }
    finish(image::diff(&mapping::diff_request(a)))
}

fn dispatch_platform(command: &str, a: &Args, backend: &uitap_platform::Current) -> ! {
    let result: OpResult<serde_json::Value> = match command {
        "doctor" => Ok(observe::doctor(backend, capture_available())),
        "screens" => observe::screens(backend),
        "windows" => observe::windows(backend, &mapping::window_query(a)),
        "frontmost" => observe::frontmost(backend),
        "shot" => image::shot(backend, &mapping::shot_request(a, "shot")).map(|out| out.json()),
        "wait-stable" => wait::wait_stable_json(backend, &mapping::target(a), &mapping::wait_params(a)),
        "click" => match a.point("at") {
            Some(at) => input::click(backend, at, mapping::button(a), a.int("count", 1).max(1) as u32),
            None => Err("--at x,y is required".into()),
        },
        "move" => match a.point("to") {
            Some(to) => input::move_to(backend, to),
            None => Err("--to x,y is required".into()),
        },
        "drag" => match (a.point("from"), a.point("to")) {
            (Some(from), Some(to)) => input::drag(
                backend,
                from,
                to,
                mapping::button(a),
                a.int("duration", 300).max(0) as u64,
            ),
            _ => Err("--from x,y and --to x,y are required".into()),
        },
        "scroll" => input::scroll(backend, &mapping::scroll_request(a)),
        "type" => match a.str("text") {
            Some(text) => input::type_text(backend, text, a.int("delay", 0).max(0) as u64),
            None => Err("--text is required".into()),
        },
        "key" => dispatch_key(a, backend),
        "activate" => input::activate(backend, &mapping::activate_request(a)),
        "tap" => match a.point("at") {
            Some(at) => tap_op::tap(backend, &mapping::tap_request(a, at)),
            None => Err("--at x,y is required".into()),
        },
        other => fail(format!("unknown command: {other}")),
    };
    finish(result)
}

fn dispatch_key(a: &Args, backend: &uitap_platform::Current) -> OpResult<serde_json::Value> {
    let combo = a.str("combo").ok_or("--combo is required")?;
    let (key_code, modifiers, _name) =
        parse_combo(combo).ok_or_else(|| format!("unrecognized combo: {combo}"))?;
    input::key(backend, combo, key_code, &modifiers, a.int("repeat", 1).max(1) as u32)
}

fn run_mcp() -> ! {
    // 主线程留给 AppKit：NSRunningApplication 的激活只有在本进程主线程执行才会落地，
    // 因此 server 跑在独立线程上，主线程专职服务这类调用。
    if !uitap_platform::start_main_thread_service() {
        fail("cannot start main-thread service");
    }

    let server = std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(e) => return Err(format!("cannot start runtime: {e}")),
        };
        runtime
            .block_on(uitap_mcp::serve())
            .map_err(|e| format!("mcp server failed: {e}"))
    });

    // 服务器线程结束时（客户端断开）让主线程循环退出。
    let _monitor = std::thread::spawn(move || match server.join() {
        Ok(Ok(())) => uitap_platform::stop_main_thread_service(),
        Ok(Err(message)) => {
            eprintln!("{message}");
            uitap_platform::stop_main_thread_service();
        }
        Err(_) => uitap_platform::stop_main_thread_service(),
    });

    uitap_platform::pump_main_thread()
}

/// 让 `emit` 在本模块可见（供上述 dispatch 使用）。
#[allow(dead_code)]
fn emit_value(value: serde_json::Value) -> ! {
    emit(value)
}
