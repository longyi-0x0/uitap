//! 全局点坐标与矩形。原点在左上角，与 macOS CGEvent、Windows 屏幕坐标一致。

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn min_x(&self) -> f64 {
        self.x
    }
    pub fn min_y(&self) -> f64 {
        self.y
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }
    pub fn area(&self) -> f64 {
        self.w * self.h
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let x0 = self.min_x().max(other.min_x());
        let y0 = self.min_y().max(other.min_y());
        let x1 = self.max_x().min(other.max_x());
        let y1 = self.max_y().min(other.max_y());
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }

    /// 向外取整到整数边界，与截屏区域取整的行为一致。
    pub fn integral(&self) -> Rect {
        let x0 = self.min_x().floor();
        let y0 = self.min_y().floor();
        let x1 = self.max_x().ceil();
        let y1 = self.max_y().ceil();
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// 像素坐标与全局点坐标之间的换算依据。
///
/// `origin` 是该图左上角对应的全局点，`scale` 是像素宽除以点宽。
/// 换算用最终图像的真实尺寸反推，因此缩放过的图依然成立。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub origin: Point,
    pub scale: f64,
}

impl Anchor {
    pub const fn new(origin: Point, scale: f64) -> Self {
        Self { origin, scale }
    }

    pub fn effective_scale(&self) -> f64 {
        if self.scale == 0.0 {
            1.0
        } else {
            self.scale
        }
    }

    pub fn to_point(&self, r: Rect) -> Rect {
        let s = self.effective_scale();
        Rect::new(
            self.origin.x + r.min_x() / s,
            self.origin.y + r.min_y() / s,
            r.w / s,
            r.h / s,
        )
    }

    pub fn to_pixel(&self, p: Point) -> Point {
        let s = self.effective_scale();
        Point::new((p.x - self.origin.x) * s, (p.y - self.origin.y) * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_integral_expands_outward() {
        let r = Rect::new(10.4, 20.6, 30.2, 40.1).integral();
        assert_eq!(r, Rect::new(10.0, 20.0, 31.0, 41.0));
    }

    #[test]
    fn anchor_roundtrip_retina() {
        // 与 Swift 版验证过的用例一致：scale=2, origin=(100,50)，点 (110,60) -> 像素 (20,20)
        let anchor = Anchor::new(Point::new(100.0, 50.0), 2.0);
        assert_eq!(anchor.to_pixel(Point::new(110.0, 60.0)), Point::new(20.0, 20.0));
    }

    #[test]
    fn anchor_to_point_scales_region() {
        let anchor = Anchor::new(Point::new(84.0, 30.0), 2.0);
        let got = anchor.to_point(Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(got, Rect::new(84.0, 30.0, 100.0, 50.0));
    }
}
