use crate::{Host, settings::Settings, works};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    pub fn valid(self) -> bool {
        (120..=18000).contains(&self.width)
            && (80..=18000).contains(&self.height)
            && self.x.abs_diff(0) < 100_000
            && self.y.abs_diff(0) < 100_000
    }
}

#[derive(Clone, PartialEq)]
pub struct Geometry {
    pub bounds: Bounds,
    pub clips: Vec<Bounds>,
    pub frontmost: bool,
    pub presentable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    pub process: String,
    pub window: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Packet {
    pub html: String,
    pub context: Value,
    pub playing: bool,
    pub clipped: bool,
    pub clip_rects: Vec<Bounds>,
}

pub struct Scene {
    label: String,
    packet: Packet,
    themes: Vec<String>,
    ready: bool,
    shown: Option<Instant>,
    created: Instant,
    target: Option<(Target, Geometry)>,
}

pub fn exists(app: &AppHandle) -> bool {
    app.state::<Host>().scene.lock().unwrap().is_some()
}

pub fn open(
    app: &AppHandle,
    settings: &Settings,
    theme: &str,
    target: Option<(Target, Geometry)>,
) -> Result<(), String> {
    let state = app.state::<Host>();
    let _guard = state.player_operation.lock().unwrap();
    if state.scene.lock().unwrap().is_some() {
        return Ok(());
    }
    let payload = works::prepare(
        &state.data_dir,
        &settings.work_id,
        &settings.locale,
        theme,
        &settings.parameters,
        false,
    )?;
    let themes = payload.themes;
    let mut context = payload.context;
    context["hostControls"] = json!({"skip": true});
    context["workId"] = json!(settings.work_id);
    let label = format!(
        "playback-{}",
        state
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    *state.scene.lock().unwrap() = Some(Scene {
        label: label.clone(),
        packet: Packet {
            html: payload.html,
            context,
            playing: false,
            clipped: cfg!(target_os = "macos"),
            clip_rects: target
                .as_ref()
                .map(|(_, geometry)| geometry.clips.clone())
                .unwrap_or_default(),
        },
        themes,
        target,
        ready: false,
        shown: None,
        created: Instant::now(),
    });
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("playback.html".into()))
        .title("ChatGPT")
        .visible(false)
        .focused(false)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .accept_first_mouse(true)
        .inner_size(920.0, 600.0)
        .on_navigation(|url| crate::local_page(url, &["/playback.html"]))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .focusable(false)
        .skip_taskbar(true)
        .always_on_top(cfg!(windows))
        .visible_on_all_workspaces(cfg!(target_os = "macos"))
        .transparent(cfg!(target_os = "macos"));
    #[cfg(windows)]
    let builder = builder.data_directory(state.data_dir.join("webview"));
    let window = match builder.build() {
        Ok(window) => window,
        Err(error) => {
            close(app, Some(&label));
            return Err(error.to_string());
        }
    };
    register_owned_window(app, &window);
    present(app, &label)
}

fn register_owned_window(app: &AppHandle, window: &WebviewWindow) {
    let sender = app.state::<Host>().observer.clone();
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        let _ = sender.send(crate::observer::Message::Owned(Some(format!(
            "0x{:x}",
            hwnd.0 as usize
        ))));
    }
    #[cfg(target_os = "macos")]
    {
        // NSWindow numbers are read on the AppKit thread.
        let handle = window.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(window_ptr) = handle.ns_window() {
                use objc2_app_kit::NSWindow;
                let native = unsafe { &*window_ptr.cast::<NSWindow>() };
                let id = native.windowNumber();
                if id > 0 {
                    let _ = sender.send(crate::observer::Message::Owned(Some(id.to_string())));
                }
            }
        });
    }
}

pub fn close(app: &AppHandle, label: Option<&str>) {
    let state = app.state::<Host>();
    let scene = {
        let mut current = state.scene.lock().unwrap();
        if label.is_some_and(|label| current.as_ref().is_none_or(|scene| scene.label != label)) {
            return;
        }
        current.take()
    };
    if let Some(scene) = scene {
        if let Some(window) = app.get_webview_window(&scene.label) {
            let _ = window.destroy();
        }
        let _ = state.observer.send(crate::observer::Message::Owned(None));
    }
}

pub fn packet(app: &AppHandle, label: &str) -> Result<Packet, String> {
    app.state::<Host>()
        .scene
        .lock()
        .unwrap()
        .as_ref()
        .filter(|scene| scene.label == label)
        .map(|scene| scene.packet.clone())
        .ok_or_else(|| "Playback session ended".into())
}

pub fn ready(app: &AppHandle, label: &str) -> Result<(), String> {
    let state = app.state::<Host>();
    if let Some(scene) = state
        .scene
        .lock()
        .unwrap()
        .as_mut()
        .filter(|scene| scene.label == label)
    {
        scene.ready = true;
    }
    present(app, label)
}

fn place(window: &WebviewWindow, bounds: Bounds) -> tauri::Result<()> {
    #[cfg(windows)]
    {
        window.set_position(tauri::PhysicalPosition::new(bounds.x, bounds.y))?;
        window.set_size(tauri::PhysicalSize::new(
            bounds.width as u32,
            bounds.height as u32,
        ))?;
    }
    #[cfg(target_os = "macos")]
    {
        window.set_position(tauri::LogicalPosition::new(
            bounds.x as f64,
            bounds.y as f64,
        ))?;
        window.set_size(tauri::LogicalSize::new(
            bounds.width as f64,
            bounds.height as f64,
        ))?;
    }
    Ok(())
}

fn present(app: &AppHandle, label: &str) -> Result<(), String> {
    let state = app.state::<Host>();
    let (target, geometry) = {
        let mut scene = state.scene.lock().unwrap();
        let Some(scene) = scene.as_mut().filter(|scene| {
            scene.label == label
                && scene.ready
                && scene.shown.is_none()
                && scene
                    .target
                    .as_ref()
                    .is_some_and(|(_, geometry)| geometry.presentable)
        }) else {
            return Ok(());
        };
        scene.shown = Some(Instant::now());
        scene.target.clone().unwrap()
    };
    let Some(window) = app.get_webview_window(label) else {
        close(app, Some(label));
        return Ok(());
    };
    let result = (|| -> tauri::Result<()> {
        apply_geometry(app, &window, &target, &geometry)?;
        {
            let mut scene = state.scene.lock().unwrap();
            if let Some(scene) = scene.as_mut().filter(|scene| scene.label == label) {
                scene.packet.playing = true;
            }
        }
        app.emit_to(label, "prism-present", ())?;
        Ok(())
    })();
    if let Err(error) = result {
        close(app, Some(label));
        return Err(error.to_string());
    }
    Ok(())
}

pub fn play_target(
    app: &AppHandle,
    settings: &Settings,
    theme: &str,
    target: Target,
    geometry: Geometry,
) -> Result<(), String> {
    let state = app.state::<Host>();
    let label = {
        let mut current = state.scene.lock().unwrap();
        match current.as_mut() {
            Some(scene) if scene.target.is_none() => {
                scene.packet.clip_rects = geometry.clips.clone();
                scene.target = Some((target.clone(), geometry.clone()));
                Some(scene.label.clone())
            }
            Some(_) => return Ok(()),
            None => None,
        }
    };
    if let Some(label) = label {
        present(app, &label)
    } else {
        open(app, settings, theme, Some((target, geometry)))
    }
}

fn apply_geometry(
    app: &AppHandle,
    window: &WebviewWindow,
    target: &Target,
    geometry: &Geometry,
) -> tauri::Result<()> {
    if !geometry.presentable {
        window.hide()?;
        return Ok(());
    }
    place(window, geometry.bounds)?;
    #[cfg(target_os = "macos")]
    app.emit_to(
        window.label(),
        "prism-geometry",
        json!({
            "clipRects": geometry.clips,
            "clipped": true
        }),
    )?;
    if !window.is_visible()? {
        window.show()?;
    }
    #[cfg(target_os = "macos")]
    {
        let handle = window.clone();
        let relative = target.window.parse::<isize>().unwrap_or_default();
        let frontmost = geometry.frontmost;
        app.run_on_main_thread(move || {
            if let Ok(pointer) = handle.ns_window() {
                use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
                let native = unsafe { &*pointer.cast::<NSWindow>() };
                native.setIgnoresMouseEvents(!frontmost);
                if frontmost {
                    native.setLevel(1000);
                } else {
                    native.setLevel(0);
                    if relative > 0 {
                        native.orderWindow_relativeTo(NSWindowOrderingMode::Above, relative);
                    }
                }
            }
        })?;
    }
    #[cfg(windows)]
    let _ = (app, target);
    Ok(())
}

pub fn update_target(app: &AppHandle, target: &Target, geometry: Option<Geometry>) -> bool {
    let state = app.state::<Host>();
    let (label, shown, changed) = {
        let mut current = state.scene.lock().unwrap();
        let Some(scene) = current.as_mut() else {
            return false;
        };
        let Some((_, previous)) = scene.target.as_mut().filter(|(key, _)| key == target) else {
            return false;
        };
        let changed = geometry
            .as_ref()
            .is_some_and(|geometry| geometry != previous);
        if let Some(geometry) = &geometry {
            scene.packet.clip_rects = geometry.clips.clone();
            *previous = geometry.clone();
        }
        (scene.label.clone(), scene.shown.is_some(), changed)
    };
    match geometry {
        None => close(app, Some(&label)),
        Some(geometry) if shown && changed => {
            if let Some(window) = app.get_webview_window(&label) {
                if apply_geometry(app, &window, target, &geometry).is_err() {
                    close(app, Some(&label));
                }
            }
        }
        Some(_) if !shown => {
            let _ = present(app, &label);
        }
        _ => {}
    }
    true
}

pub fn close_process(app: &AppHandle, process: &str) {
    let state = app.state::<Host>();
    let label = state
        .scene
        .lock()
        .unwrap()
        .as_ref()
        .filter(|scene| {
            scene
                .target
                .as_ref()
                .is_some_and(|(target, _)| target.process == process)
        })
        .map(|scene| scene.label.clone());
    if let Some(label) = label {
        close(app, Some(&label));
    }
}

pub fn update_theme(app: &AppHandle, theme: &str) {
    let state = app.state::<Host>();
    let (label, context) = {
        let mut current = state.scene.lock().unwrap();
        let Some(scene) = current
            .as_mut()
            .filter(|scene| scene.packet.context["theme"] != theme)
        else {
            return;
        };
        let context = if scene.themes.iter().any(|value| value == theme) {
            scene.packet.context["theme"] = json!(theme);
            Some(scene.packet.context.clone())
        } else {
            None
        };
        (scene.label.clone(), context)
    };
    if let Some(context) = context {
        let _ = app.emit_to(
            label.as_str(),
            "prism-theme",
            json!({ "theme": theme, "context": context }),
        );
    } else {
        close(app, Some(&label));
    }
}

pub fn expire(app: &AppHandle) {
    let state = app.state::<Host>();
    let label = state
        .scene
        .lock()
        .unwrap()
        .as_ref()
        .filter(|scene| {
            (!scene.ready && scene.created.elapsed() > Duration::from_secs(10))
                || scene
                    .shown
                    .is_some_and(|time| time.elapsed() > Duration::from_secs(45))
        })
        .map(|scene| scene.label.clone());
    if let Some(label) = label {
        close(app, Some(&label));
    }
}
