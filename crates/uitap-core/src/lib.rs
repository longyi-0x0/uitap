//! uitap 的平台无关内核。
//!
//! 这里只有三类东西：几何与锚点换算、位图与差分、以及平台后端的 trait 与共享类型。
//! 任何平台调用都不出现在本 crate，因此它能在任意目标上编译与测试。

pub mod backend;
pub mod geom;
pub mod jsonout;
pub mod pixels;
pub mod store;
