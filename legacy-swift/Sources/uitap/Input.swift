import CoreGraphics
import Foundation

func sleepMs(_ ms: Int) {
    guard ms > 0 else { return }
    usleep(useconds_t(min(ms, 60_000) * 1000))
}

private func eventSource() -> CGEventSource? {
    CGEventSource(stateID: .hidSystemState)
}

/// 合成事件投递到 HID 层，与真实输入设备的路径一致，需要辅助功能授权。
private func post(_ event: CGEvent?) {
    event?.post(tap: .cghidEventTap)
}

func moveMouse(to point: CGPoint) {
    let source = eventSource()
    post(CGEvent(mouseEventSource: source,
                 mouseType: .mouseMoved,
                 mouseCursorPosition: point,
                 mouseButton: .left))
}

private func buttonEventType(_ button: MouseButton, down: Bool, dragging: Bool = false) -> CGEventType {
    switch button {
    case .left: return dragging ? .leftMouseDragged : (down ? .leftMouseDown : .leftMouseUp)
    case .right: return dragging ? .rightMouseDragged : (down ? .rightMouseDown : .rightMouseUp)
    case .middle: return dragging ? .otherMouseDragged : (down ? .otherMouseDown : .otherMouseUp)
    }
}

enum MouseButton: String {
    case left, right, middle

    var cgButton: CGMouseButton {
        switch self {
        case .left: return .left
        case .right: return .right
        case .middle: return .center
        }
    }
}

private func mouseEvent(type: CGEventType, at point: CGPoint, button: MouseButton, clickState: Int64 = 1) {
    let source = eventSource()
    guard let event = CGEvent(mouseEventSource: source,
                              mouseType: type,
                              mouseCursorPosition: point,
                              mouseButton: button.cgButton)
    else { return }
    if clickState > 1 {
        event.setIntegerValueField(.mouseEventClickState, value: clickState)
    }
    post(event)
}

func clickMouse(at point: CGPoint, button: MouseButton, count: Int, holdMs: Int) {
    moveMouse(to: point)
    sleepMs(40)
    let clicks = max(1, count)
    for index in 1...clicks {
        let state = Int64(index)
        mouseEvent(type: buttonEventType(button, down: true), at: point, button: button, clickState: state)
        sleepMs(max(10, holdMs))
        mouseEvent(type: buttonEventType(button, down: false), at: point, button: button, clickState: state)
        if index < clicks { sleepMs(70) }
    }
}

func dragMouse(from start: CGPoint, to end: CGPoint, button: MouseButton, durationMs: Int, steps: Int) {
    moveMouse(to: start)
    sleepMs(60)
    mouseEvent(type: buttonEventType(button, down: true), at: start, button: button)
    let segments = max(2, steps)
    let perStep = max(4, durationMs / segments)
    for index in 1...segments {
        let t = Double(index) / Double(segments)
        let point = CGPoint(x: start.x + (end.x - start.x) * t,
                            y: start.y + (end.y - start.y) * t)
        mouseEvent(type: buttonEventType(button, down: true, dragging: true), at: point, button: button)
        sleepMs(perStep)
    }
    sleepMs(40)
    mouseEvent(type: buttonEventType(button, down: false), at: end, button: button)
}

func scrollWheel(at point: CGPoint?, dy: Int, dx: Int) {
    if let p = point {
        moveMouse(to: p)
        sleepMs(40)
    }
    let source = eventSource()
    guard let event = CGEvent(scrollWheelEvent2Source: source,
                              units: .pixel,
                              wheelCount: 2,
                              wheel1: Int32(dy),
                              wheel2: Int32(dx),
                              wheel3: 0)
    else { return }
    post(event)
}

/// 逐段投递 unicode 字符串。`delayMs` 为 0 时整串一次投递。
func typeText(_ text: String, delayMs: Int) {
    let source = eventSource()
    let units = Array(text.utf16)

    func send(_ slice: ArraySlice<UInt16>) {
        var buffer = Array(slice)
        let count = buffer.count
        guard count > 0 else { return }
        buffer.withUnsafeMutableBufferPointer { raw in
            let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true)
            down?.keyboardSetUnicodeString(stringLength: count, unicodeString: raw.baseAddress!)
            post(down)
            let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false)
            up?.keyboardSetUnicodeString(stringLength: count, unicodeString: raw.baseAddress!)
            post(up)
        }
    }

    guard delayMs > 0 else {
        // 单次事件过长会被系统丢弃，按 20 个 UTF-16 单元分段。
        var offset = 0
        while offset < units.count {
            let end = min(offset + 20, units.count)
            send(units[offset..<end])
            offset = end
            if offset < units.count { sleepMs(2) }
        }
        return
    }

    var index = 0
    while index < units.count {
        let scalar = units[index]
        // 代理对需要两个单元一起投递。
        let take = (scalar >= 0xD800 && scalar <= 0xDBFF && index + 1 < units.count) ? 2 : 1
        send(units[index..<(index + take)])
        index += take
        sleepMs(delayMs)
    }
}

func pressCombo(_ keyCode: CGKeyCode, flags: CGEventFlags) {
    let source = eventSource()
    let down = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: true)
    down?.flags = flags
    post(down)
    sleepMs(20)
    let up = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: false)
    up?.flags = flags
    post(up)
}
