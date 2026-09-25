//! 输出约定：单行 JSON 到 stdout；错误也是 JSON，退出码 1。

use serde_json::Value;

use uitap_core::jsonout::to_line;

pub fn emit(value: Value) -> ! {
    println!("{}", to_line(&value));
    std::process::exit(0)
}

pub fn fail(message: impl AsRef<str>) -> ! {
    println!("{}", to_line(&uitap_ops::json::error_value(message)));
    std::process::exit(1)
}

/// 操作层的 `Result` 直接转成「打印并退出」。
pub fn finish(result: uitap_ops::OpResult<Value>) -> ! {
    match result {
        Ok(value) => emit(value),
        Err(message) => fail(message),
    }
}
