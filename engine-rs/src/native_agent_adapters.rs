//! Assistant format recognition for extension target safety.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterKind {
    Claude,
    Codex,
}

pub(crate) fn kind_for_bin(bin: &str) -> Option<AdapterKind> {
    match std::path::Path::new(bin)
        .file_stem()
        .and_then(|name| name.to_str())
    {
        Some("claude") => Some(AdapterKind::Claude),
        Some("codex") => Some(AdapterKind::Codex),
        _ => None,
    }
}
