//! Explicit Linux AppImage link registration; inspection never creates files.
#[cfg(any(target_os = "linux", all(unix, test)))]
mod linux;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Preview {
    pub base: String,
    pub executable: String,
    pub entry: String,
    pub current_handler: String,
    pub registered: bool,
}

#[cfg(target_os = "linux")]
pub use linux::Registration;
