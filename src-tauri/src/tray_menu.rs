use crate::{Host, reconcile_library_playback, settings::Settings, works};
use tauri::{
    AppHandle, Emitter, Manager, Wry,
    menu::{CheckMenuItem, Menu, MenuItem},
};

struct TrayMenu {
    startup: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

fn labels(locale: &str) -> (&'static str, &'static str) {
    if locale == "zh" {
        ("启动动画", "退出 Prism")
    } else {
        ("Startup animation", "Quit Prism")
    }
}

pub fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let settings = app.state::<Host>().settings.lock().unwrap().clone();
    let (startup_label, quit_label) = labels(&settings.locale);
    let startup = CheckMenuItem::with_id(
        app,
        "toggle-startup",
        startup_label,
        !settings.work_id.is_empty(),
        settings.enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&startup, &quit])?;
    app.manage(TrayMenu { startup, quit });
    Ok(menu)
}

pub fn sync(app: &AppHandle, settings: &Settings) {
    if let Some(menu) = app.try_state::<TrayMenu>() {
        let (startup, quit) = labels(&settings.locale);
        let _ = menu.startup.set_text(startup);
        let _ = menu.startup.set_enabled(!settings.work_id.is_empty());
        let _ = menu.startup.set_checked(settings.enabled);
        let _ = menu.quit.set_text(quit);
    }
}

pub fn toggle(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _operation = host.settings_operation.lock().unwrap();
        let mut settings = host.settings.lock().unwrap().clone();
        if !settings.enabled
            && !works::list(&host.data_dir)
                .is_ok_and(|works| works.iter().any(|work| work.id == settings.work_id))
        {
            sync(&app, &settings);
            return;
        }
        settings.enabled = !settings.enabled;
        if settings.save(&host.data_dir).is_err() {
            settings.enabled = !settings.enabled;
            sync(&app, &settings);
            return;
        }
        *host.settings.lock().unwrap() = settings.clone();
        sync(&app, &settings);
        let _ = app.emit_to("main", "prism-startup-enabled", ());
        reconcile_library_playback(&app, &host);
    });
}
