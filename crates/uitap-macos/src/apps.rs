//! 应用匹配。
//!
//! 按名字或 bundle id 找运行中的应用。bundle id 只取最后一段做比较，
//! 这样 `Finder` 能对上 `com.apple.finder`，而不会误命中
//! `com.apple.SafariPlatformSupport.Helper` 这类把目标词放在中间段的辅助进程。

/// 命中的优先级。精确命中先于模糊命中，先遍历到的先于后遍历到的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// 名字或 bundle 末段完全相等。
    Exact,
    /// 名字包含查询词，或 bundle 末段包含查询词。
    Partial,
}

/// bundle id 的最后一段，如 `com.apple.finder` → `finder`。
pub fn bundle_last_segment(bundle_id: &str) -> &str {
    bundle_id.rsplit('.').next().unwrap_or("")
}

/// 判断一个候选是否命中。`needle` 必须已经小写化。
///
/// `prohibited` 表示该进程的激活策略阻止它成为前台应用：这类进程即使名字匹配也不能选，
/// 否则激活必然无声失败。
pub fn classify(needle: &str, localized_name: &str, bundle_id: &str, prohibited: bool) -> Option<MatchKind> {
    if needle.is_empty() || prohibited {
        return None;
    }

    let name = localized_name.to_ascii_lowercase();
    let bundle = bundle_id.to_ascii_lowercase();
    let segment = bundle_last_segment(&bundle);

    if name == needle || bundle == needle || segment == needle {
        return Some(MatchKind::Exact);
    }
    if name.contains(needle) || segment.contains(needle) {
        return Some(MatchKind::Partial);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_name_matches_via_bundle_segment() {
        // 本地化名是「访达」，只能靠 bundle 末段对上。
        assert_eq!(
            classify("finder", "访达", "com.apple.finder", false),
            Some(MatchKind::Exact)
        );
        assert_eq!(
            classify("safari", "Safari浏览器", "com.apple.Safari", false),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn helpers_never_match_exactly() {
        // 末段是 helper、中间段才含 safari —— 精确匹配不会命中它。
        assert_eq!(
            classify("safari", "自动填充 (Sourcetree)", "com.apple.SafariPlatformSupport.Helper", false),
            None
        );
        // 名字里确实带 safari 的辅助进程会落到模糊档，但同屏的 Safari 自身是精确档，
        // 精确优先，所以它不会被选中。
        assert_eq!(
            classify("safari", "Safari浏览器 Networking", "com.apple.WebKit.Networking", false),
            Some(MatchKind::Partial)
        );
        assert_eq!(
            classify("safari", "Safari浏览器", "com.apple.Safari", false),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn exact_precedence_is_a_lower_rank_than_partial() {
        // MatchKind 的序保证「精确」排在「模糊」之前，供调用方取最小值。
        assert!(MatchKind::Exact < MatchKind::Partial);
    }

    #[test]
    fn localized_name_still_works() {
        assert_eq!(
            classify("访达", "访达", "com.apple.finder", false),
            Some(MatchKind::Exact)
        );
        assert_eq!(
            classify("文本编辑", "文本编辑", "com.apple.TextEdit", false),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn names_match_per_their_own_form() {
        // 名字完全相等即精确，不必依赖 bundle。
        assert_eq!(
            classify("code", "Code", "com.microsoft.VSCode", false),
            Some(MatchKind::Exact)
        );
        assert_eq!(
            classify("sourcetree", "Sourcetree", "com.torusknot.SourceTreeNotMAS", false),
            Some(MatchKind::Exact)
        );
        // 名字只是包含时算模糊。
        assert_eq!(
            classify("code", "Code – Insiders", "com.microsoft.VSCodeInsiders", false),
            Some(MatchKind::Partial)
        );
    }

    #[test]
    fn prohibited_apps_are_refused() {
        assert_eq!(classify("finder", "访达", "com.apple.finder", true), None);
    }

    #[test]
    fn unrelated_apps_do_not_match() {
        assert_eq!(classify("finder", "Code", "com.microsoft.VSCode", false), None);
        assert_eq!(classify("", "Code", "com.microsoft.VSCode", false), None);
    }

    #[test]
    fn bundle_without_dots_is_its_own_segment() {
        assert_eq!(bundle_last_segment("finder"), "finder");
        assert_eq!(bundle_last_segment(""), "");
        assert_eq!(bundle_last_segment("a.b.c"), "c");
    }
}
