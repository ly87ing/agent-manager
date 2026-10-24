//! Exact local credential namespaces; no token data is requested or returned.
use crate::Context;
#[cfg(target_os = "macos")]
use anyhow::bail;
use anyhow::Result;

#[cfg(target_os = "macos")]
mod keychain;

pub(super) struct Snapshot {
    #[cfg(target_os = "macos")]
    items: Vec<keychain::Item>,
    pub skipped: bool,
}

impl Snapshot {
    pub fn count(&self) -> usize {
        #[cfg(target_os = "macos")]
        {
            self.items.len()
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }
    pub fn fingerprint(&self) -> Vec<u8> {
        let bytes = vec![u8::from(self.skipped)];
        #[cfg(target_os = "macos")]
        {
            let mut bytes = bytes;
            for item in &self.items {
                bytes.extend(item.fingerprint());
            }
            bytes
        }
        #[cfg(not(target_os = "macos"))]
        bytes
    }
    pub fn clear(
        &self,
        mut before: impl FnMut() -> Result<()>,
        mut completed: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        for item in &self.items {
            before()?;
            keychain::remove(item)?;
            completed()?;
        }
        let _ = &mut completed;
        let _ = &mut before;
        Ok(())
    }
}

pub(super) fn inspect(ctx: &Context, files_only: bool) -> Result<Snapshot> {
    #[cfg(target_os = "macos")]
    {
        if files_only {
            return Ok(Snapshot {
                items: vec![],
                skipped: true,
            });
        }
        for name in [
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_SECURESTORAGE_CONFIG_DIR",
            "CLAUDE_CODE_CUSTOM_OAUTH_URL",
        ] {
            if std::env::var_os(name).is_some() {
                bail!("Custom Claude credential namespaces are not included; choose file-only cleanup or inspect the default environment separately.");
            }
        }
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buffer = vec![0u8; 64 * 1024];
        let mut found = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                &mut record,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() || record.pw_dir.is_null() || record.pw_name.is_null() {
            bail!("The native Keychain user could not be verified.");
        }
        let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }.to_str()?;
        if dunce::canonicalize(&ctx.home_dir)? != dunce::canonicalize(home)? {
            bail!("An isolated HOME cannot inspect the real user's Keychain; choose file-only cleanup.");
        }
        let native_name = unsafe { std::ffi::CStr::from_ptr(record.pw_name) }.to_str()?;
        let candidate = std::env::var("USER")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| native_name.to_owned());
        let account = if !candidate.is_empty()
            && candidate
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            candidate
        } else {
            "claude-code-user".into()
        };
        Ok(Snapshot {
            items: keychain::inspect(&account)?,
            skipped: false,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (ctx, files_only);
        Ok(Snapshot { skipped: false })
    }
}
