use agent_manager_engine::desktop_links::Preview;

#[cfg(target_os = "linux")]
fn registration(
    app: &tauri::AppHandle,
) -> Result<Option<agent_manager_engine::desktop_links::Registration>, String> {
    use tauri::Manager;
    app.env()
        .appimage
        .map(|path| {
            agent_manager_engine::desktop_links::Registration::new(
                path.into(),
                &app.config().identifier,
            )
            .map_err(|error| error.to_string())
        })
        .transpose()
}

pub(crate) fn preview(app: &tauri::AppHandle) -> Result<Option<Preview>, String> {
    #[cfg(target_os = "linux")]
    return registration(app)?
        .map(|value| value.preview().map_err(|error| error.to_string()))
        .transpose();
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Ok(None)
    }
}

pub(crate) fn register(app: &tauri::AppHandle, base: &str) -> Result<Preview, String> {
    #[cfg(target_os = "linux")]
    return registration(app)?
        .ok_or("Explicit registration is available for Linux AppImages only.")?
        .register(base)
        .map_err(|error| error.to_string());
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, base);
        Err("Explicit registration is available for Linux AppImages only.".into())
    }
}
