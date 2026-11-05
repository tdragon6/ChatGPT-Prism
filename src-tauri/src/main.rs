#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod catalog;
#[cfg(target_os = "macos")]
mod macos;
mod observer;
mod platform;
mod playback;
mod settings;
mod tray_menu;
mod works;

use observer::{Message, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use settings::{CloseBehavior, Settings};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

struct Host {
    data_dir: PathBuf,
    settings: Mutex<Settings>,
    runtime: Mutex<Runtime>,
    scene: Mutex<Option<playback::Scene>>,
    observer: Sender<Message>,
    sequence: AtomicU64,
    player_operation: Mutex<()>,
    settings_operation: Mutex<()>,
    catalog_operation: Mutex<()>,
    startup_update: Mutex<Option<catalog::Update>>,
    close_requested: Mutex<bool>,
    quitting: AtomicBool,
}

#[derive(Serialize)]
struct ManagerState {
    #[serde(rename = "defaultMarketUrl")]
    default_market_url: &'static str,
    #[serde(rename = "startupUpdate")]
    startup_update: Option<catalog::Update>,
    settings: Settings,
    works: Vec<works::Work>,
    runtime: Runtime,
    #[serde(rename = "closeRequested")]
    close_requested: bool,
}

fn main_window(window: &WebviewWindow) -> Result<(), String> {
    (window.label() == "main")
        .then_some(())
        .ok_or_else(|| "Invalid manager window".into())
}

fn playback_window(window: &WebviewWindow) -> Result<&str, String> {
    window
        .label()
        .starts_with("playback-")
        .then_some(window.label())
        .ok_or_else(|| "Invalid playback window".into())
}

fn manager_state(host: &Host) -> Result<ManagerState, String> {
    let settings = host.settings.lock().unwrap().clone();
    let works = works::list(&host.data_dir)?;
    let runtime = host.runtime.lock().unwrap().clone();
    let close_requested = *host.close_requested.lock().unwrap();
    Ok(ManagerState {
        default_market_url: catalog::DEFAULT_MARKET_URL,
        startup_update: host.startup_update.lock().unwrap().clone(),
        settings,
        works,
        runtime,
        close_requested,
    })
}

#[tauri::command]
fn get_state(window: WebviewWindow, host: State<'_, Host>) -> Result<ManagerState, String> {
    main_window(&window)?;
    manager_state(&host)
}

#[tauri::command]
fn get_runtime(window: WebviewWindow, host: State<'_, Host>) -> Result<Runtime, String> {
    main_window(&window)?;
    Ok(host.runtime.lock().unwrap().clone())
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let auto = app.autolaunch();
    if enabled {
        auto.enable()
    } else {
        auto.disable()
    }
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_settings(
    window: WebviewWindow,
    app: AppHandle,
    mut settings: Settings,
    update_enabled: Option<bool>,
) -> Result<ManagerState, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _lock = host.settings_operation.lock().unwrap();
        settings.market_url = settings.market_url.trim().to_owned();
        if settings.market_url.is_empty() {
            settings.market_url = catalog::DEFAULT_MARKET_URL.into();
        }
        let current = host.settings.lock().unwrap().clone();
        if update_enabled == Some(false) {
            settings.enabled = current.enabled;
        }
        settings.validate()?;
        let installed = works::list(&host.data_dir)?;
        if settings.work_id.is_empty() {
            if settings.enabled || settings.parameters != json!({}) {
                return Err("Select a startup animation first".into());
            }
        } else {
            let work = installed
                .iter()
                .find(|work| work.id == settings.work_id)
                .ok_or("Animation not found")?;
            works::validate_parameters(work, &settings.parameters)?;
        }
        settings
            .work_parameters
            .retain(|id, _| id != &settings.work_id && installed.iter().any(|work| work.id == *id));
        for (id, values) in &settings.work_parameters {
            let work = installed.iter().find(|work| work.id == *id).unwrap();
            works::validate_parameters(work, values)?;
        }
        if !settings.application_path.is_empty()
            && settings.application_path != current.application_path
        {
            settings.application_path = platform::discover(&settings.application_path)?
                .path
                .to_string_lossy()
                .into_owned();
        }
        let autostart_changed = settings.launch_at_login != current.launch_at_login;
        if autostart_changed {
            apply_autostart(&app, settings.launch_at_login)?;
        }
        if let Err(error) = settings.save(&host.data_dir) {
            if autostart_changed {
                let _ = apply_autostart(&app, current.launch_at_login);
            }
            return Err(error);
        }
        #[cfg(target_os = "macos")]
        if settings.appearance != current.appearance {
            let _ = macos::set_manager_appearance(&window, &settings.appearance);
        }
        if (settings.enabled, &settings.work_id, &settings.locale)
            != (current.enabled, &current.work_id, &current.locale)
        {
            tray_menu::sync(&app, &settings);
        }
        *host.settings.lock().unwrap() = settings;
        let _ = host.observer.send(Message::SettingsChanged);
        manager_state(&host)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn select_application(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<String>, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let picker = app.dialog().file().set_title("ChatGPT");
        #[cfg(windows)]
        let chosen = picker
            .add_filter("Windows application", &["exe"])
            .blocking_pick_file();
        #[cfg(target_os = "macos")]
        let chosen = picker
            .set_directory("/Applications")
            .add_filter("macOS application", &["app"])
            .blocking_pick_file();
        chosen
            .map(|file| {
                let path = file.into_path().map_err(|error| error.to_string())?;
                platform::discover(&path.to_string_lossy())
                    .map(|client| client.path.to_string_lossy().into_owned())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn import_work(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<ManagerState>, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(folder) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let host = app.state::<Host>();
        let _lock = host.settings_operation.lock().unwrap();
        let folder = folder.into_path().map_err(|error| error.to_string())?;
        works::import_work(&host.data_dir, &folder)?;
        manager_state(&host).map(Some)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn unload_work(
    window: WebviewWindow,
    app: AppHandle,
    work_id: String,
) -> Result<ManagerState, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _lock = host.settings_operation.lock().unwrap();
        let mut state = manager_state(&host)?;
        if !state.works.iter().any(|work| work.id == work_id) {
            return Err("Animation not found".into());
        }
        if state.settings.work_id == work_id && state.settings.enabled {
            state.settings.enabled = false;
            state.settings.save(&host.data_dir)?;
            *host.settings.lock().unwrap() = state.settings.clone();
            tray_menu::sync(&app, &state.settings);
            reconcile_library_playback(&app, &host);
        }
        Ok(state)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn reconcile_library_playback(app: &AppHandle, host: &Host) {
    let (reply, finished) = mpsc::channel();
    if host.observer.send(Message::SyncSettings(reply)).is_err()
        || finished.recv_timeout(Duration::from_secs(2)).is_err()
    {
        playback::close(app, None);
    }
}

#[tauri::command]
async fn remove_work(
    window: WebviewWindow,
    app: AppHandle,
    work_id: String,
) -> Result<ManagerState, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _lock = host.settings_operation.lock().unwrap();
        let mut settings = host.settings.lock().unwrap().clone();
        let mut installed = works::list(&host.data_dir)?;
        let index = installed
            .iter()
            .position(|work| work.id == work_id)
            .ok_or("Animation not found")?;
        installed.remove(index);
        let mut playback_changed = false;
        let save_selection = || {
            let active_removed = settings.work_id == work_id;
            let configured = settings.work_parameters.remove(&work_id).is_some();
            if active_removed {
                settings.reset_work_selection(&installed);
            }
            if active_removed || configured {
                settings.save(&host.data_dir)?;
                if active_removed {
                    tray_menu::sync(&app, &settings);
                }
                *host.settings.lock().unwrap() = settings;
                playback_changed = active_removed;
            }
            Ok(())
        };
        let removed = works::remove_work(&host.data_dir, &work_id, save_selection);
        if removed.is_ok() {
            catalog::evict_preview(&work_id);
        }
        if playback_changed {
            reconcile_library_playback(&app, &host);
        }
        removed?;
        manager_state(&host)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_studio_preview(
    window: WebviewWindow,
    app: AppHandle,
    work_id: String,
    theme: String,
    locale: String,
    parameters: Value,
) -> Result<works::Playback, String> {
    main_window(&window)?;
    if !matches!(locale.as_str(), "zh" | "en") || !matches!(theme.as_str(), "light" | "dark") {
        return Err("Invalid preview appearance".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let mut prepared =
            works::prepare(&host.data_dir, &work_id, &locale, &theme, &parameters, true)?;
        prepared.context["workId"] = json!(work_id);
        Ok(prepared)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_market_preview(
    window: WebviewWindow,
    app: AppHandle,
    work_id: String,
    theme: String,
    locale: String,
    parameters: Value,
) -> Result<works::Playback, String> {
    main_window(&window)?;
    if !matches!(locale.as_str(), "zh" | "en") || !matches!(theme.as_str(), "light" | "dark") {
        return Err("Invalid preview appearance".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let source = host.settings.lock().unwrap().market_url.clone();
        let mut prepared = catalog::preview(
            &host.data_dir,
            &source,
            &work_id,
            &locale,
            &theme,
            &parameters,
        )?;
        if host.settings.lock().unwrap().market_url != source {
            return Err("The marketplace source changed. Open the preview again.".into());
        }
        prepared.context["workId"] = json!(work_id);
        Ok(prepared)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_catalog(window: WebviewWindow, app: AppHandle) -> Result<catalog::Catalog, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _operation = host.catalog_operation.lock().unwrap();
        let source = host.settings.lock().unwrap().market_url.clone();
        catalog::get(&host.data_dir, &source)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn refresh_catalog(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<catalog::Catalog, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _operation = host.catalog_operation.lock().unwrap();
        let source = host.settings.lock().unwrap().market_url.clone();
        catalog::refresh(&host.data_dir, &source)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn install_catalog_work(
    window: WebviewWindow,
    app: AppHandle,
    work_id: String,
) -> Result<ManagerState, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        let _operation = host.catalog_operation.lock().unwrap();
        let source = host.settings.lock().unwrap().market_url.clone();
        catalog::install(&host.data_dir, &source, &work_id)?;
        let _settings = host.settings_operation.lock().unwrap();
        let mut selected = host.settings.lock().unwrap().clone();
        let active = selected.work_id == work_id;
        if let Some(parameters) = if active {
            Some(&mut selected.parameters)
        } else {
            selected.work_parameters.get_mut(&work_id)
        } {
            let work = works::list(&host.data_dir)?
                .into_iter()
                .find(|work| work.id == work_id)
                .ok_or("Animation not found")?;
            if works::validate_parameters(&work, parameters).is_err() {
                *parameters = json!({});
                selected.save(&host.data_dir)?;
                *host.settings.lock().unwrap() = selected;
            }
        }
        if active {
            let _ = host.observer.send(Message::WorkChanged);
        }
        manager_state(&host)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn check_app_update(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<catalog::Update, String> {
    main_window(&window)?;
    let version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || catalog::check_app_update(&version))
        .await
        .map_err(|error| error.to_string())?
}

fn open_web(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("/usr/bin/open");
    #[cfg(windows)]
    let mut command = Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
            .join(r"System32\cmd.exe"),
    );
    #[cfg(windows)]
    platform::quiet(&mut command).args(["/D", "/C", "start", ""]);
    let status = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("Could not open the browser".into());
    }
    Ok(())
}

#[tauri::command]
fn open_repository(window: WebviewWindow) -> Result<(), String> {
    main_window(&window)?;
    open_web(catalog::PROJECT_URL)
}

#[tauri::command]
fn open_releases(window: WebviewWindow, version: Option<String>) -> Result<(), String> {
    main_window(&window)?;
    let url = match version {
        Some(version) => catalog::release_url(&version)?,
        None => format!("{}/releases", catalog::PROJECT_URL),
    };
    open_web(&url)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CloseAction {
    Background,
    Quit,
    Cancel,
}

#[tauri::command]
async fn respond_close(
    window: WebviewWindow,
    app: AppHandle,
    action: CloseAction,
    remember: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<Host>();
        {
            let _operation = host.settings_operation.lock().unwrap();
            let mut requested = host.close_requested.lock().unwrap();
            if !*requested {
                return Ok(());
            }
            if remember && !matches!(action, CloseAction::Cancel) {
                let mut settings = host.settings.lock().unwrap().clone();
                settings.close_behavior = match action {
                    CloseAction::Background => CloseBehavior::Background,
                    CloseAction::Quit => CloseBehavior::Quit,
                    CloseAction::Cancel => unreachable!(),
                };
                settings.save(&host.data_dir)?;
                *host.settings.lock().unwrap() = settings;
            }
            *requested = false;
        }
        app.run_on_main_thread({
            let app = app.clone();
            move || match action {
                CloseAction::Background => hide_manager(&app),
                CloseAction::Quit => quit(&app, 0),
                CloseAction::Cancel => {}
            }
        })
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn launch_chatgpt(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (reply, result) = mpsc::channel();
        app.state::<Host>()
            .observer
            .send(Message::Launch(reply))
            .map_err(|error| error.to_string())?;
        result
            .recv_timeout(Duration::from_secs(24))
            .map_err(|error| error.to_string())?
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn get_playback(window: WebviewWindow, app: AppHandle) -> Result<playback::Packet, String> {
    playback::packet(&app, playback_window(&window)?)
}

#[tauri::command]
fn playback_ready(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    playback::ready(&app, playback_window(&window)?)
}

#[tauri::command]
fn finish_playback(window: WebviewWindow, app: AppHandle, reason: String) -> Result<(), String> {
    let label = playback_window(&window)?;
    if !matches!(reason.as_str(), "complete" | "skip" | "failed") {
        return Err("Invalid playback result".into());
    }
    playback::close(&app, Some(label));
    Ok(())
}

fn local_page(url: &tauri::Url, paths: &[&str]) -> bool {
    if url.scheme() == "about" {
        return matches!(url.path(), "blank" | "srcdoc");
    }
    let local = match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "https" | "http" => {
            (url.host_str() == Some("tauri.localhost") && url.port().is_none())
                || (cfg!(debug_assertions)
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(1420))
        }
        _ => false,
    };
    local && url.username().is_empty() && url.password().is_none() && paths.contains(&url.path())
}

fn data_dir(_app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let directory = std::env::current_exe()?
        .parent()
        .ok_or("Missing application directory")?
        .join("data");
    #[cfg(target_os = "macos")]
    let directory = _app.path().app_data_dir()?;
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        let is_link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let is_link = {
            use std::os::windows::fs::MetadataExt;
            is_link || metadata.file_attributes() & 0x0400 != 0
        };
        if is_link || !metadata.is_dir() {
            return Err("Invalid application data directory".into());
        }
    }
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn show_manager(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        if !window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false) {
            let _ = app.set_dock_visibility(true);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to("main", "prism-manager-visibility", true);
    }
}

fn hide_manager(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = app.emit_to("main", "prism-manager-visibility", false);
        let _ = window.hide();
        #[cfg(target_os = "macos")]
        let _ = app.set_dock_visibility(false);
    }
}

fn request_manager_close(app: &AppHandle, reveal: bool) {
    let host = app.state::<Host>();
    let behavior = host.settings.lock().unwrap().close_behavior;
    match behavior {
        CloseBehavior::Background => hide_manager(app),
        CloseBehavior::Quit => quit(app, 0),
        CloseBehavior::Ask => {
            if reveal {
                show_manager(app);
            }
            let was_requested = std::mem::replace(&mut *host.close_requested.lock().unwrap(), true);
            if !was_requested {
                let _ = app.emit_to("main", "prism-close-request", ());
            }
        }
    }
}

fn quit(app: &AppHandle, code: i32) {
    let Some(host) = app.try_state::<Host>() else {
        app.exit(code);
        return;
    };
    if host.quitting.swap(true, Ordering::AcqRel) {
        return;
    }
    let (reply, finished) = mpsc::channel();
    let _ = host.observer.send(Message::Stop(Some(reply)));
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = finished.recv_timeout(Duration::from_secs(1));
        app.exit(code);
    });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let directory = data_dir(app.handle())?;
    #[cfg(target_os = "macos")]
    let packaged = app.path().resource_dir()?.join("startup");
    #[cfg(windows)]
    let packaged = directory.join("startup");
    let packaged_works = works::seed_packaged(&directory, &packaged)?;
    let (sender, receiver) = mpsc::channel();
    let mut settings = Settings::load(&directory);
    let saved_settings = settings.clone();
    settings.migrate_work_ids(&packaged_works.aliases);
    settings.launch_at_login = app
        .autolaunch()
        .is_enabled()
        .unwrap_or(settings.launch_at_login);
    let installed = works::list(&directory)?
        .into_iter()
        .filter(|work| !packaged_works.aliases.contains_key(&work.id))
        .collect::<Vec<_>>();
    settings.resolve_installed_works(&installed, packaged_works.default_id.as_deref());
    if settings != saved_settings || !directory.join("settings.json").exists() {
        if settings != saved_settings {
            packaged_works.preserve_settings(&directory)?;
        }
        settings.save(&directory)?;
    }
    packaged_works.archive_legacy(&directory)?;
    let launch_hidden = std::env::args_os().any(|argument| argument == "--tray");
    #[cfg(target_os = "macos")]
    let appearance = settings.appearance.clone();
    #[cfg(windows)]
    let initial_surface = if settings.appearance == "light" {
        tauri::window::Color(247, 247, 244, 255)
    } else {
        tauri::window::Color(8, 9, 8, 255)
    };
    let (width, height) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map_or((1100.0, 760.0), |monitor| {
            let size = monitor
                .work_area()
                .size
                .to_logical::<f64>(monitor.scale_factor());
            (
                1100.0_f64.min(size.width - 24.0),
                760.0_f64.min(size.height - 24.0),
            )
        });
    app.manage(Host {
        data_dir: directory.clone(),
        settings: Mutex::new(settings),
        runtime: Mutex::new(Runtime::default()),
        scene: Mutex::new(None),
        observer: sender.clone(),
        sequence: AtomicU64::new(1),
        player_operation: Mutex::new(()),
        settings_operation: Mutex::new(()),
        catalog_operation: Mutex::new(()),
        startup_update: Mutex::new(None),
        close_requested: Mutex::new(false),
        quitting: AtomicBool::new(false),
    });
    #[cfg(target_os = "macos")]
    macos::install_quit_handler(app.handle())?;
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("ChatGPT Prism")
        .always_on_top(false)
        .inner_size(width, height)
        .min_inner_size(700.0_f64.min(width), 580.0_f64.min(height))
        .on_navigation(|url| local_page(url, &["", "/", "/index.html"]))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .visible(!launch_hidden)
        .center();
    #[cfg(windows)]
    let builder = builder
        .background_color(initial_surface)
        .data_directory(directory.join("webview"));
    let main = builder.build()?;
    #[cfg(target_os = "macos")]
    macos::set_manager_appearance(&main, &appearance)?;
    if launch_hidden {
        main.hide()?;
        #[cfg(target_os = "macos")]
        app.set_dock_visibility(false);
    }

    let menu = tray_menu::menu(app.handle())?;
    let icon = Image::from_bytes(if cfg!(target_os = "macos") {
        include_bytes!("../icons/tray-mac.png")
    } else {
        include_bytes!("../icons/icon.png")
    })?;
    let tray = TrayIconBuilder::with_id("prism")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle-startup" => tray_menu::toggle(app),
            "quit" => quit(app, 0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_manager(tray.app_handle());
            }
        })
        .build(app)?;
    #[cfg(target_os = "macos")]
    tray.with_inner_tray_icon(|tray| {
        let mtm = objc2::MainThreadMarker::new().unwrap();
        if let Some(button) = tray.ns_status_item().and_then(|item| item.button(mtm))
            && let Some(icon) = button.image()
        {
            let mut size = icon.size();
            size.width = 20.0;
            size.height = 20.0;
            icon.setSize(size);
        }
    })?;
    #[cfg(windows)]
    let _ = tray;

    let handle = app.handle().clone();
    std::thread::spawn(move || observer::run(handle, receiver, sender));
    start_update_tasks(app.handle());
    Ok(())
}

fn start_update_tasks(app: &AppHandle) {
    let settings = app.state::<Host>().settings.lock().unwrap().clone();
    if settings.refresh_market_on_startup {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let host = app.state::<Host>();
            let _operation = host.catalog_operation.lock().unwrap();
            let _ = catalog::refresh(&host.data_dir, &settings.market_url);
            let _ = app.emit_to(
                "main",
                "prism-market-refreshed",
                json!({ "source": settings.market_url }),
            );
        });
    }
    if settings.check_updates_on_startup {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let Ok(update) = catalog::check_app_update(&app.package_info().version.to_string())
            else {
                return;
            };
            let host = app.state::<Host>();
            let settings = host.settings.lock().unwrap();
            if update.available
                && settings.check_updates_on_startup
                && settings.ignored_update_version.as_deref()
                    != Some(update.latest_version.as_str())
            {
                *host.startup_update.lock().unwrap() = Some(update.clone());
                let _ = app.emit_to("main", "prism-startup-update", update);
            }
        });
    }
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_manager(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--tray"]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_runtime,
            save_settings,
            select_application,
            import_work,
            unload_work,
            remove_work,
            get_studio_preview,
            get_market_preview,
            get_catalog,
            refresh_catalog,
            install_catalog_work,
            check_app_update,
            open_repository,
            open_releases,
            respond_close,
            launch_chatgpt,
            get_playback,
            playback_ready,
            finish_playback
        ])
        .setup(setup)
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if window.label() == "main" && matches!(event, WindowEvent::Resized(_)) {
                // WebView2 does not see its parent window being minimized.
                let visible =
                    window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false);
                let _ = window.emit_to("main", "prism-manager-visibility", visible);
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "main" {
                    request_manager_close(window.app_handle(), false);
                } else {
                    playback::close(window.app_handle(), Some(window.label()));
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("start ChatGPT Prism");
    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            if app
                .try_state::<Host>()
                .is_some_and(|host| !host.quitting.load(Ordering::Acquire))
            {
                api.prevent_exit();
                quit(app, code.unwrap_or(0));
            }
        }
        tauri::RunEvent::Exit => {
            if let Some(host) = app.try_state::<Host>() {
                let _ = host.observer.send(Message::Stop(None));
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => show_manager(app),
        _ => {}
    });
}
