use crate::{Host, settings::Settings, works};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const WINDOW_GRACE: Duration = Duration::from_millis(1200);

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

    fn similar_size(self, previous: Self) -> bool {
        self.width * 4 >= previous.width * 3 && self.height * 4 >= previous.height * 3
    }
}

#[derive(Clone, PartialEq)]
pub struct Geometry {
    pub bounds: Bounds,
    pub clips: Vec<Bounds>,
    pub frontmost: bool,
    pub presentable: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Target {
    pub process: String,
    pub window: String,
}

pub struct WindowState {
    pub geometry: Option<Geometry>,
    pub preferred: bool,
    pub z_order: usize,
}

#[derive(Clone)]
struct Session {
    target: Target,
    geometry: Geometry,
    handoff_until: Instant,
    unavailable_until: Option<Instant>,
}

impl Session {
    fn update(&mut self, windows: &HashMap<Target, WindowState>, now: Instant) {
        let current = windows
            .get(&self.target)
            .and_then(|window| window.geometry.as_ref());
        if now < self.handoff_until {
            let replacement = windows
                .iter()
                .filter(|(target, window)| {
                    target.process == self.target.process
                        && **target != self.target
                        && window.preferred
                })
                .filter_map(|(target, window)| {
                    let geometry = window.geometry.as_ref()?;
                    (geometry.presentable
                        && geometry.frontmost
                        && geometry.bounds.similar_size(self.geometry.bounds))
                    .then_some((target, geometry, window.z_order))
                })
                .min_by_key(|(_, _, order)| *order);
            if let Some((target, geometry, _)) = replacement {
                self.target.clone_from(target);
                self.geometry.clone_from(geometry);
                self.unavailable_until = None;
                return;
            }
        }

        if let Some(geometry) = current {
            self.geometry.clone_from(geometry);
            if geometry.presentable || cfg!(target_os = "macos") {
                self.unavailable_until = None;
                return;
            }
        }
        self.geometry.presentable = false;
        self.geometry.clips.clear();
        self.unavailable_until.get_or_insert(now + WINDOW_GRACE);
    }
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
    session: Option<Session>,
}

pub fn exists(app: &AppHandle) -> bool {
    app.state::<Host>().scene.lock().unwrap().is_some()
}

pub fn open(app: &AppHandle, settings: &Settings, theme: &str) -> Result<(), String> {
    create_scene(app, settings, theme, None).map(|_| ())
}

fn create_scene(
    app: &AppHandle,
    settings: &Settings,
    theme: &str,
    session: Option<Session>,
) -> Result<bool, String> {
    let state = app.state::<Host>();
    let _guard = state.player_operation.lock().unwrap();
    if state.scene.lock().unwrap().is_some() {
        return Ok(false);
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
            clip_rects: session
                .as_ref()
                .map(|session| session.geometry.clips.clone())
                .unwrap_or_default(),
        },
        themes,
        session,
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
    if let Err(error) = present(app, &label) {
        close(app, Some(&label));
        return Err(error);
    }
    Ok(true)
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
    let handle = app.clone();
    let label = label.to_owned();
    app.run_on_main_thread(move || {
        let state = handle.state::<Host>();
        let (target, geometry, starting) = {
            let mut current = state.scene.lock().unwrap();
            let Some(scene) = current
                .as_mut()
                .filter(|scene| scene.label == label && scene.ready)
            else {
                return;
            };
            let Some(session) = &scene.session else {
                return;
            };
            let starting = scene.shown.is_none();
            if starting {
                if !session.geometry.presentable {
                    return;
                }
                scene.shown = Some(Instant::now());
                scene.packet.playing = true;
            }
            (session.target.clone(), session.geometry.clone(), starting)
        };
        let Some(window) = handle.get_webview_window(&label) else {
            close(&handle, Some(&label));
            return;
        };
        let result = apply_geometry(&handle, &window, &target, &geometry).and_then(|()| {
            if starting {
                handle.emit_to(label.as_str(), "prism-present", ())?;
            }
            Ok(())
        });
        if result.is_err() {
            close(&handle, Some(&label));
        }
    })
    .map_err(|error| error.to_string())
}

pub fn play_target(
    app: &AppHandle,
    settings: &Settings,
    theme: &str,
    target: Target,
    geometry: Geometry,
    handoff_until: Instant,
) -> Result<bool, String> {
    let state = app.state::<Host>();
    let session = Session {
        target,
        geometry,
        handoff_until,
        unavailable_until: None,
    };
    let label = {
        let mut current = state.scene.lock().unwrap();
        match current.as_mut() {
            Some(scene) if scene.session.is_none() => {
                scene.packet.clip_rects.clone_from(&session.geometry.clips);
                scene.session = Some(session.clone());
                scene.created = Instant::now();
                Some(scene.label.clone())
            }
            Some(_) => return Ok(false),
            None => None,
        }
    };
    if let Some(label) = label {
        if let Err(error) = present(app, &label) {
            close(app, Some(&label));
            return Err(error);
        }
        Ok(true)
    } else {
        create_scene(app, settings, theme, Some(session))
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
    #[cfg(windows)]
    if !window.is_visible()? {
        window.show()?;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
        let pointer = window.ns_window()?;
        let native = unsafe { &*pointer.cast::<NSWindow>() };
        native.setIgnoresMouseEvents(!geometry.frontmost);
        if geometry.frontmost {
            native.setLevel(1000);
            native.orderFrontRegardless();
        } else if let Some(relative) = target.window.parse::<isize>().ok().filter(|id| *id > 0) {
            native.setLevel(0);
            native.orderWindow_relativeTo(NSWindowOrderingMode::Above, relative);
        } else {
            window.hide()?;
        }
    }
    #[cfg(windows)]
    let _ = (app, target);
    Ok(())
}

pub fn update_target(app: &AppHandle, windows: &HashMap<Target, WindowState>) {
    let state = app.state::<Host>();
    let label = {
        let mut current = state.scene.lock().unwrap();
        let Some(scene) = current.as_mut() else {
            return;
        };
        let Some(session) = scene.session.as_mut() else {
            return;
        };
        let previous = session.clone();
        session.update(windows, Instant::now());
        if session.target == previous.target && session.geometry == previous.geometry {
            return;
        }
        scene.packet.clip_rects.clone_from(&session.geometry.clips);
        scene.label.clone()
    };
    if present(app, &label).is_err() {
        close(app, Some(&label));
    }
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
                .session
                .as_ref()
                .is_some_and(|session| session.target.process == process)
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
            scene.session.as_ref().is_some_and(|session| {
                session
                    .unavailable_until
                    .is_some_and(|deadline| Instant::now() >= deadline)
            }) || ((!scene.ready || (scene.session.is_some() && scene.shown.is_none()))
                && scene.created.elapsed() > Duration::from_secs(10))
                || scene
                    .shown
                    .is_some_and(|time| time.elapsed() > Duration::from_secs(45))
        })
        .map(|scene| scene.label.clone());
    if let Some(label) = label {
        close(app, Some(&label));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, width: i32, preferred: bool) -> (Target, WindowState) {
        let bounds = Bounds {
            x: 50,
            y: 50,
            width,
            height: 600,
        };
        (
            Target {
                process: "123:start".into(),
                window: id.into(),
            },
            WindowState {
                geometry: Some(Geometry {
                    bounds,
                    clips: vec![Bounds {
                        x: 0,
                        y: 0,
                        ..bounds
                    }],
                    frontmost: true,
                    presentable: true,
                }),
                preferred,
                z_order: usize::from(!preferred),
            },
        )
    }

    fn session(now: Instant) -> Session {
        let (target, window) = window("old", 800, true);
        Session {
            target,
            geometry: window.geometry.unwrap(),
            handoff_until: now + Duration::from_secs(15),
            unavailable_until: None,
        }
    }

    #[test]
    fn foreground_replacement_takes_over_while_original_is_open() {
        let now = Instant::now();
        let mut session = session(now);
        let (replacement, mut state) = window("new", 700, true);
        state.geometry.as_mut().unwrap().bounds.x = 400;
        let windows = HashMap::from([window("old", 800, false), (replacement, state)]);
        session.update(&windows, now);
        assert_eq!(session.target.window, "new");
        assert_eq!(session.geometry.bounds.width, 700);
        assert!(session.unavailable_until.is_none());
    }

    #[test]
    fn replacement_resumes_after_gap() {
        let now = Instant::now();
        let mut session = session(now);
        session.update(&HashMap::new(), now);
        assert!(!session.geometry.presentable);
        assert_eq!(session.unavailable_until, Some(now + WINDOW_GRACE));

        let windows = HashMap::from([window("new", 800, true)]);
        session.update(&windows, now + Duration::from_millis(200));
        assert_eq!(session.target.window, "new");
        assert!(session.geometry.presentable);
        assert!(session.unavailable_until.is_none());
    }

    #[test]
    fn popup_does_not_replace_existing_main_window() {
        let now = Instant::now();
        let mut session = session(now);
        let windows = HashMap::from([window("old", 800, false), window("popup", 300, true)]);
        session.update(&windows, now);
        assert_eq!(session.target.window, "old");
    }

    #[test]
    fn handoff_requires_same_process_instance_within_startup_period() {
        let now = Instant::now();
        let mut session = session(now);
        let (mut other, window_state) = window("new", 800, true);
        other.process = "123:later-start".into();
        session.update(&HashMap::from([(other, window_state)]), now);
        assert_eq!(session.target.window, "old");
        assert!(!session.geometry.presentable);

        let windows = HashMap::from([window("new", 800, true)]);
        session.update(&windows, session.handoff_until);
        assert_eq!(session.target.window, "old");
        assert!(!session.geometry.presentable);
    }
}
