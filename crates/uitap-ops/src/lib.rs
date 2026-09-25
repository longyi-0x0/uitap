//! 操作层：把后端调用编排成 JSON 契约。
//!
//! 这一层只依赖 `Backend` trait，因此平台无关。CLI 与 MCP server 都调用它，
//! JSON 结构只在这里定义一次。

pub mod image;
pub mod input;
pub mod json;
pub mod observe;
pub mod tap;
pub mod types;
pub mod wait;

pub use types::{
    ActivateRequest, AnchorOverride, CropRequest, DiffRequest, OpResult, PixelRequest, ScrollRequest,
    ShotOutcome, ShotRequest, TapRequest, Units, WaitParams, WindowQuery,
};
