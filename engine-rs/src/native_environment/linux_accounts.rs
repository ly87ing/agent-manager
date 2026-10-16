//! Existing-service, current-UID primary locale evidence.
use serde::Serialize;

fn locale(value: &str) -> Option<String> {
    super::Target {
        locale: Some(value.into()),
        ..Default::default()
    }
    .validate()
    .ok()?;
    Some(value.into())
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub(super) struct Primary {
    pub(super) locale: String,
    pub(super) uid: u32,
    pub(super) bus: String,
    pub(super) owner: String,
    pub(super) path: String,
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<Primary> {
    use std::time::Duration;
    zbus::block_on(futures_lite::future::race(
        async {
            let connection = zbus::Connection::system().await.ok()?;
            let bus = zbus::Proxy::new(
                &connection,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .await
            .ok()?;
            let id: String = bus.call("GetId", &()).await.ok()?;
            let owner: String = bus
                .call("GetNameOwner", &("org.freedesktop.Accounts",))
                .await
                .ok()?;
            let accounts = zbus::Proxy::new(
                &connection,
                owner.as_str(),
                "/org/freedesktop/Accounts",
                "org.freedesktop.Accounts",
            )
            .await
            .ok()?;
            let uid = unsafe { libc::geteuid() };
            let path: zbus::zvariant::OwnedObjectPath = accounts
                .call_with_flags(
                    "FindUserById",
                    zbus::proxy::MethodFlags::NoAutoStart.into(),
                    &(uid as i64,),
                )
                .await
                .ok()??;
            if !path.as_str().starts_with("/org/freedesktop/Accounts/User") {
                return None;
            }
            let properties = zbus::Proxy::new(
                &connection,
                owner.as_str(),
                path.as_str(),
                "org.freedesktop.DBus.Properties",
            )
            .await
            .ok()?;
            async fn get(
                properties: &zbus::Proxy<'_>,
                key: &str,
            ) -> Option<zbus::zvariant::OwnedValue> {
                properties
                    .call_with_flags(
                        "Get",
                        zbus::proxy::MethodFlags::NoAutoStart.into(),
                        &("org.freedesktop.Accounts.User", key),
                    )
                    .await
                    .ok()?
            }
            if u64::try_from(&get(&properties, "Uid").await?).ok()? != uid as u64 {
                return None;
            }
            let value = get(&properties, "Language").await?;
            let locale = locale(<&str>::try_from(&value).ok()?)?;
            if u64::try_from(&get(&properties, "Uid").await?).ok()? != uid as u64 {
                return None;
            }
            let after_id: String = bus.call("GetId", &()).await.ok()?;
            let after_owner: String = bus
                .call("GetNameOwner", &("org.freedesktop.Accounts",))
                .await
                .ok()?;
            (id == after_id && owner == after_owner).then_some(Primary {
                locale,
                uid,
                bus: id,
                owner,
                path: path.to_string(),
            })
        },
        async {
            async_io::Timer::after(Duration::from_secs(8)).await;
            None
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_locale_keeps_countryless_and_unmapped_formats_evidence() {
        for value in ["en", "en_US.UTF-8", "C.UTF-8", "sr_RS@latin"] {
            assert_eq!(locale(value).as_deref(), Some(value));
        }
        for value in ["", "en_US;touch /tmp/unexpected", "$(unexpected)"] {
            assert!(locale(value).is_none());
        }
    }
}
