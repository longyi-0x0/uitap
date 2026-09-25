//! CoreGraphics / ApplicationServices 的最小 FFI 面。
//!
//! 只声明本项目真正用到的函数，避免引入整包绑定带来的 API 漂移。

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code)]

use std::ffi::c_void;

// ---- AXValue 承载的类型 ----
pub const kAXValueTypeCGPoint: u32 = 1;
pub const kAXValueTypeCGSize: u32 = 2;
pub const kAXValueTypeCGRect: u32 = 3;

pub type CGDirectDisplayID = u32;
pub type CGWindowID = u32;
pub type CGWindowListOption = u32;
pub type CGImageRef = *mut c_void;
pub type CFArrayRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFNumberRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFTypeRef = *const c_void;
pub type CGEventRef = *mut c_void;
pub type CGEventSourceRef = *mut c_void;
pub type CGEventType = u32;
pub type CGMouseButton = u32;
pub type CGScrollEventUnit = u32;
pub type CGEventField = u32;
pub type CFRunLoopRef = *mut c_void;
pub type AXUIElementRef = *mut c_void;
pub type CFBooleanRef = *const c_void;
pub type CFIndex = isize;
pub type CFTypeID = usize;
pub type AXValueRef = *mut c_void;
pub type CFRunLoopMode = CFStringRef;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

impl CGRect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            origin: CGPoint { x, y },
            size: CGSize {
                width: w,
                height: h,
            },
        }
    }

    /// CG 的 CGRectNull：原点为正无穷、尺寸为零。
    pub const NULL: Self = Self {
        origin: CGPoint {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
}

// ---- 窗口列表选项 ----
pub const kCGWindowListOptionAll: CGWindowListOption = 0;
pub const kCGWindowListOptionOnScreenOnly: CGWindowListOption = 1 << 0;
pub const kCGWindowListOptionIncludingWindow: CGWindowListOption = 1 << 3;
pub const kCGWindowListExcludeDesktopElements: CGWindowListOption = 1 << 4;
pub const kCGNullWindowID: CGWindowID = 0;

// ---- 事件 ----
pub const kCGHIDEventTap: u32 = 0;
pub const kCGEventSourceStateHIDSystemState: i32 = 1;
pub const kCGScrollEventUnitPixel: CGScrollEventUnit = 0;

pub const kCGEventMouseMoved: CGEventType = 5;
pub const kCGEventLeftMouseDown: CGEventType = 1;
pub const kCGEventLeftMouseUp: CGEventType = 2;
pub const kCGEventRightMouseDown: CGEventType = 3;
pub const kCGEventRightMouseUp: CGEventType = 4;
pub const kCGEventLeftMouseDragged: CGEventType = 6;
pub const kCGEventRightMouseDragged: CGEventType = 7;
pub const kCGEventOtherMouseDown: CGEventType = 25;
pub const kCGEventOtherMouseUp: CGEventType = 26;
pub const kCGEventOtherMouseDragged: CGEventType = 27;

pub const kCGMouseButtonLeft: CGMouseButton = 0;
pub const kCGMouseButtonRight: CGMouseButton = 1;
pub const kCGMouseButtonCenter: CGMouseButton = 2;

/// kCGMouseEventClickState
pub const kCGMouseEventClickState: CGEventField = 1;

// ---- 修饰键位 ----
pub const kCGEventFlagMaskShift: u64 = 1 << 17;
pub const kCGEventFlagMaskControl: u64 = 1 << 18;
pub const kCGEventFlagMaskAlternate: u64 = 1 << 19;
pub const kCGEventFlagMaskCommand: u64 = 1 << 20;
pub const kCGEventFlagMaskSecondaryFn: u64 = 1 << 23;

// ---- CoreFoundation ----
pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kCFNumberSInt64Type: i32 = 4;
pub const kCFNumberFloat64Type: i32 = 6;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGMainDisplayID() -> CGDirectDisplayID;
    pub fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    pub fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    pub fn CGDisplayPixelsWide(display: CGDirectDisplayID) -> usize;
    pub fn CGDisplayPixelsHigh(display: CGDirectDisplayID) -> usize;
    pub fn CGPreflightScreenCaptureAccess() -> bool;

    pub fn CGWindowListCopyWindowInfo(option: CGWindowListOption, relative_to: CGWindowID) -> CFArrayRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXIsProcessTrusted() -> bool;

    /// 取某个进程的辅助功能根元素。
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    /// 设置元素属性，返回 AXError（0 为成功）。
    pub fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    /// 执行元素支持的动作（AXPress、AXIncrement 等），返回 AXError。
    pub fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    /// 取元素支持的动作名列表，调用方负责释放。
    pub fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> i32;
    /// 取元素所属进程。
    pub fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    /// 取屏幕某点下的元素，调用方负责释放。
    pub fn AXUIElementCopyElementAtPosition(
        application: AXUIElementRef,
        x: f32,
        y: f32,
        element: *mut AXUIElementRef,
    ) -> i32;

    /// 解出 AXValue 承载的几何值（CGPoint / CGSize / CGRect）。
    pub fn AXValueGetValue(value: AXValueRef, the_type: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    /// 默认 run loop mode 的全局常量。
    pub static kCFRunLoopDefaultMode: CFRunLoopMode;
    pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    /// 让出控制权一小段时间，期间处理系统派发到本进程的事件。
    pub fn CFRunLoopRunInMode(mode: CFRunLoopMode, seconds: f64, return_after_source_handled: bool) -> i32;
    pub fn CFRelease(cf: CFTypeRef);
    /// 取 +1 引用。
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    /// 两个 CF 对象是否等价（用于比较 AXUIElement 身份）。
    pub fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> bool;
    pub fn CFArrayGetCount(array: CFArrayRef) -> isize;
    pub fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> CFTypeRef;
    pub fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> CFTypeRef;
    pub fn CFNumberGetValue(number: CFNumberRef, the_type: i32, value: *mut c_void) -> bool;
    pub fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut u8,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
    /// CFBoolean 的真值与假值。
    pub static kCFBooleanTrue: CFBooleanRef;
    pub static kCFBooleanFalse: CFBooleanRef;
    pub fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const u8,
        encoding: u32,
    ) -> CFStringRef;

    /// 类型判定。AX 属性返回的 CFTypeRef 必须先问类型再取值。
    pub fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    pub fn CFStringGetTypeID() -> CFTypeID;
    pub fn CFArrayGetTypeID() -> CFTypeID;
    pub fn CFNumberGetTypeID() -> CFTypeID;
    pub fn CFBooleanGetTypeID() -> CFTypeID;
    pub fn AXValueGetTypeID() -> CFTypeID;
    pub fn CFBooleanGetValue(boolean: CFBooleanRef) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGEventSourceCreate(state_id: i32) -> CGEventSourceRef;
    pub fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: CGEventType,
        position: CGPoint,
        button: CGMouseButton,
    ) -> CGEventRef;
    pub fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    pub fn CGEventCreateScrollWheelEvent(
        source: CGEventSourceRef,
        units: CGScrollEventUnit,
        wheel_count: u32,
        ...
    ) -> CGEventRef;
    pub fn CGEventPost(tap: u32, event: CGEventRef);
    pub fn CGEventSetFlags(event: CGEventRef, flags: u64);
    pub fn CGEventSetIntegerValueField(event: CGEventRef, field: CGEventField, value: i64);
    pub fn CGEventKeyboardSetUnicodeString(
        event: CGEventRef,
        length: usize,
        unicode_string: *const u16,
    );
}

/// 用 UTF-8 字符串构造一个 CFString，调用方负责释放。
pub unsafe fn cfstring(text: &str) -> CFStringRef {
    let mut bytes: Vec<u8> = text.as_bytes().to_vec();
    bytes.push(0);
    CFStringCreateWithCString(std::ptr::null(), bytes.as_ptr(), kCFStringEncodingUTF8)
}

/// 从字典按名称取键。`key` 生命周期由调用方保证，返回值为借用。
pub unsafe fn dict_value(dict: CFDictionaryRef, key: CFStringRef) -> CFTypeRef {
    CFDictionaryGetValue(dict, key as *const c_void)
}

pub unsafe fn number_i64(number: CFNumberRef) -> Option<i64> {
    if number.is_null() {
        return None;
    }
    let mut out: i64 = 0;
    if CFNumberGetValue(number, kCFNumberSInt64Type, &mut out as *mut i64 as *mut c_void) {
        Some(out)
    } else {
        None
    }
}

pub unsafe fn number_f64(number: CFNumberRef) -> Option<f64> {
    if number.is_null() {
        return None;
    }
    let mut out: f64 = 0.0;
    if CFNumberGetValue(number, kCFNumberFloat64Type, &mut out as *mut f64 as *mut c_void) {
        Some(out)
    } else {
        None
    }
}

pub unsafe fn string_value(string: CFStringRef) -> Option<String> {
    if string.is_null() {
        return None;
    }
    let mut buffer = vec![0u8; 4096];
    let ok = CFStringGetCString(
        string,
        buffer.as_mut_ptr(),
        buffer.len() as isize,
        kCFStringEncodingUTF8,
    );
    if !ok {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

/// 读取 `kCGWindowBounds` 那种以 X/Y/Width/Height 表示的 CFDictionary。
pub unsafe fn rect_from_dict(dict: CFDictionaryRef) -> Option<CGRect> {
    if dict.is_null() {
        return None;
    }
    let read = |name: &str| -> Option<f64> {
        let key = cfstring(name);
        let value = dict_value(dict, key);
        CFRelease(key as CFTypeRef);
        number_f64(value as CFNumberRef)
    };
    let x = read("X")?;
    let y = read("Y")?;
    let w = read("Width")?;
    let h = read("Height")?;
    Some(CGRect::new(x, y, w, h))
}
