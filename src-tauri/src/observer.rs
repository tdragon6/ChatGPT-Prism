use crate::{
    Host,
    platform::{self, Client},
    playback::{self, Bounds, Geometry, Target},
    settings::Settings,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, Theme};

const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/window-host"));

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub theme: String,
    pub theme_source: String,
    pub connected: bool,
    pub running: bool,
    pub client_name: Option<String>,
    pub error: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            theme: String::new(),
            theme_source: String::new(),
            connected: false,
            running: false,
            client_name: None,
            error: None,
        }
    }
}

pub enum Message {
    SettingsChanged,
    SyncSettings(Sender<()>),
    WorkChanged,
    Owned(Option<String>),
    Launch(Sender<Result<(), String>>),
    Native(u64, Value),
    Closed(u64),
    Stop(Option<Sender<()>>),
}

struct Helper {
    child: Child,
    _folder: tempfile::TempDir,
}

impl Helper {
    fn start(client: &Client, generation: u64, sender: Sender<Message>) -> Result<Self, String> {
        let folder = tempfile::Builder::new()
            .prefix("prism-window-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let name = if cfg!(windows) {
            "PrismWindowHost.exe"
        } else {
            "PrismWindowHost"
        };
        let path = folder.path().join(name);
        std::fs::write(&path, HELPER).map_err(|error| error.to_string())?;
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        let mut command = Command::new(path);
        command
            .arg("--allow")
            .arg(&client.executable)
            .arg("--parent-pid")
            .arg(std::process::id().to_string());
        #[cfg(target_os = "macos")]
        command.arg("--bundle").arg(&client.path);
        let mut child = platform::quiet(&mut command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Could not start window listener")?;
        thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                let Ok(size) = (&mut input).take(64 * 1024).read_until(b'\n', &mut line) else {
                    break;
                };
                if size == 0 || line.last() != Some(&b'\n') {
                    break;
                }
                if let Ok(event) = serde_json::from_slice::<Value>(&line)
                    && sender.send(Message::Native(generation, event)).is_err()
                {
                    return;
                }
            }
            let _ = sender.send(Message::Closed(generation));
        });
        Ok(Self {
            child,
            _folder: folder,
        })
    }

    fn owned(&mut self, window: Option<&str>) {
        let ids: Vec<_> = window.into_iter().collect();
        let payload = if cfg!(windows) {
            json!({"type": "owned-windows", "ids": ids})
        } else {
            json!({"type": "OWNED_WINDOWS", "windowIds": ids})
        };
        if let Some(stdin) = &mut self.child.stdin {
            let _ = writeln!(stdin, "{payload}");
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"stop\n");
        }
        for _ in 0..12 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowEvent {
    pid: u32,
    #[serde(default)]
    process_start_file_time: String,
    #[serde(default)]
    process_started_at: String,
    #[serde(default)]
    hwnd: String,
    #[serde(default)]
    window_id: String,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    minimized: bool,
    #[serde(default)]
    cloaked: bool,
    #[serde(default)]
    foreground: bool,
    #[serde(default)]
    frontmost: bool,
    #[serde(default)]
    overlay_eligible: bool,
    #[serde(default)]
    clip_rects: Vec<Bounds>,
    #[serde(default)]
    rect: Option<Bounds>,
    #[serde(default)]
    bounds: Option<Bounds>,
    #[serde(default)]
    baseline: bool,
    event_name: String,
}

impl WindowEvent {
    fn process(&self) -> Option<String> {
        let start = if cfg!(windows) {
            &self.process_start_file_time
        } else {
            &self.process_started_at
        };
        (self.pid != 0 && !start.is_empty()).then(|| format!("{}:{}", self.pid, start))
    }

    fn geometry(&self) -> Option<Geometry> {
        let bounds = if cfg!(windows) {
            self.rect
        } else {
            self.bounds
        }?;
        if !bounds.valid() || !self.visible || self.minimized || self.cloaked {
            return None;
        }
        let frontmost = if cfg!(windows) {
            self.foreground
        } else {
            self.frontmost
        };
        let full = self.clip_rects.iter().any(|clip| {
            clip.x <= 0 && clip.y <= 0 && clip.width >= bounds.width && clip.height >= bounds.height
        });
        if cfg!(windows) && !(self.overlay_eligible && frontmost && full) {
            return None;
        }
        Some(Geometry {
            bounds,
            clips: self.clip_rects.clone(),
            frontmost,
            presentable: self.overlay_eligible && !self.clip_rects.is_empty(),
        })
    }
}

struct Monitor {
    settings: Settings,
    runtime: Runtime,
    client: Option<Client>,
    helper: Option<Helper>,
    generation: u64,
    owned: Option<String>,
    connect_at: Instant,
    update_theme_at: Instant,
    helper_started_at: Instant,
    last_warm: Instant,
    launch: Vec<Sender<Result<(), String>>>,
    // A process can trigger only once. Baseline processes never trigger.
    seen: HashMap<String, Option<Instant>>,
    processes: HashSet<String>,
    windows: HashMap<String, String>,
}

impl Monitor {
    fn new(settings: Settings) -> Self {
        let now = Instant::now();
        Self {
            settings,
            runtime: Runtime::default(),
            client: None,
            helper: None,
            generation: 0,
            owned: None,
            connect_at: now,
            update_theme_at: now,
            helper_started_at: now,
            last_warm: now,
            launch: Vec::new(),
            seen: HashMap::new(),
            processes: HashSet::new(),
            windows: HashMap::new(),
        }
    }

    fn publish(&self, app: &AppHandle) {
        let state = app.state::<Host>();
        let mut runtime = state.runtime.lock().unwrap();
        if *runtime != self.runtime {
            *runtime = self.runtime.clone();
            drop(runtime);
            let _ = app.emit_to("main", "prism-runtime", &self.runtime);
        }
    }

    fn disconnect(&mut self, app: &AppHandle, error: Option<String>) {
        playback::close(app, None);
        self.generation = self.generation.wrapping_add(1);
        self.helper.take();
        self.runtime.connected = false;
        self.runtime.running = false;
        self.runtime.error = error;
        self.seen.clear();
        self.processes.clear();
        self.windows.clear();
        self.owned = None;
        self.connect_at = Instant::now() + Duration::from_secs(8);
        self.publish(app);
    }

    fn connect(&mut self, app: &AppHandle, sender: &Sender<Message>) {
        self.connect_at = Instant::now() + Duration::from_secs(8);
        self.client = match platform::discover(&self.settings.application_path) {
            Ok(client) => Some(client),
            Err(error) => {
                self.runtime.error = Some(error);
                self.runtime.client_name = None;
                self.publish(app);
                self.complete_launch();
                return;
            }
        };
        let client = self.client.as_ref().unwrap();
        self.runtime.client_name = Some(client.name.clone());
        self.generation = self.generation.wrapping_add(1);
        match Helper::start(client, self.generation, sender.clone()) {
            Ok(helper) => {
                self.helper = Some(helper);
                self.helper_started_at = Instant::now();
                self.runtime.error = None;
            }
            Err(error) => {
                self.runtime.error = Some(error);
                self.complete_launch();
            }
        }
        self.update_theme_at = Instant::now();
        self.publish(app);
    }

    fn complete_launch(&mut self) {
        if self.launch.is_empty() {
            return;
        }
        let result = self
            .client
            .as_ref()
            .ok_or_else(|| {
                self.runtime
                    .error
                    .clone()
                    .unwrap_or_else(|| "ChatGPT application not found".into())
            })
            .and_then(platform::launch);
        for reply in self.launch.drain(..) {
            let _ = reply.send(result.clone());
        }
    }

    fn handle_event(&mut self, app: &AppHandle, event: Value) {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "READY" {
            let valid = event["protocolVersion"].as_u64() == Some(1)
                && (!cfg!(windows) || event["hooksRegistered"].as_u64() == Some(3));
            if valid {
                self.runtime.connected = true;
                self.runtime.error = None;
                if let Some(helper) = &mut self.helper {
                    helper.owned(self.owned.as_deref());
                }
                self.publish(app);
                self.complete_launch();
            } else {
                self.disconnect(app, Some("Unsupported window-listener protocol".into()));
                self.complete_launch();
            }
            return;
        }
        if !matches!(kind, "process" | "window") {
            return;
        }
        let is_process = kind == "process";
        let Ok(event) = serde_json::from_value::<WindowEvent>(event) else {
            return;
        };
        let Some(process) = event.process() else {
            return;
        };
        let baseline = event.baseline || event.event_name == "BASELINE" || !self.runtime.connected;
        if event.event_name == "EXIT" {
            self.processes.remove(&process);
            self.seen.remove(&process);
            self.windows.retain(|_, owner| owner != &process);
            playback::close_process(app, &process);
        } else if is_process {
            self.processes.insert(process.clone());
            self.seen
                .entry(process)
                .or_insert_with(|| (!baseline).then(Instant::now));
        } else {
            let id = if cfg!(windows) {
                event.hwnd.clone()
            } else {
                event.window_id.clone()
            };
            if id.is_empty() {
                return;
            }
            let target = Target {
                process: process.clone(),
                window: id.clone(),
            };
            let destroying = event.event_name == "DESTROY";
            let geometry = if destroying { None } else { event.geometry() };
            if destroying {
                self.windows.remove(&id);
            } else {
                self.windows.insert(id, process.clone());
            }
            let eligible = self
                .seen
                .entry(process)
                .or_insert_with(|| (!baseline).then(Instant::now));
            if !playback::update_target(app, &target, geometry.clone())
                && self.settings.enabled
                && let Some(geometry) = geometry.filter(|geometry| geometry.presentable)
                && eligible.is_some_and(|start| start.elapsed() <= Duration::from_secs(15))
            {
                *eligible = None;
                if let Err(error) = playback::play_target(
                    app,
                    &self.settings,
                    &self.runtime.theme,
                    target,
                    geometry,
                ) {
                    self.runtime.error = Some(error);
                }
            }
        }
        self.runtime.running =
            !self.windows.is_empty() || (!cfg!(windows) && !self.processes.is_empty());
        self.publish(app);
    }

    fn settings_changed(&mut self, app: &AppHandle) {
        let next = app.state::<Host>().settings.lock().unwrap().clone();
        let reconnect = self.settings.application_path != next.application_path;
        if reconnect {
            self.disconnect(app, None);
            self.client = None;
            self.runtime.client_name = None;
            self.connect_at = Instant::now();
        }
        if reconnect
            || self.settings.locale != next.locale
            || self.settings.enabled != next.enabled
            || self.settings.work_id != next.work_id
            || self.settings.parameters != next.parameters
        {
            self.animation_changed(app);
        }
        self.settings = next;
    }

    fn animation_changed(&mut self, app: &AppHandle) {
        playback::close(app, None);
        self.last_warm = Instant::now() - Duration::from_secs(2);
    }

    fn tick(&mut self, app: &AppHandle, sender: &Sender<Message>) {
        let now = Instant::now();
        if self.helper.is_none() && now >= self.connect_at {
            self.connect(app, sender);
        }
        if self.helper.is_some()
            && !self.runtime.connected
            && self.helper_started_at.elapsed() > Duration::from_secs(5)
        {
            self.disconnect(app, Some("Window listener timed out".into()));
            self.complete_launch();
        }
        if now >= self.update_theme_at {
            let system_dark = app
                .get_webview_window("main")
                .and_then(|window| window.theme().ok())
                == Some(Theme::Dark);
            let (theme, source) = platform::client_theme(self.client.as_ref(), system_dark);
            self.runtime.theme = theme.into();
            self.runtime.theme_source = source.into();
            playback::update_theme(app, theme);
            self.publish(app);
            self.update_theme_at = now + Duration::from_secs(1);
        }
        playback::expire(app);
        if self.runtime.connected
            && self.settings.enabled
            && !playback::exists(app)
            && self.last_warm.elapsed() >= Duration::from_secs(2)
        {
            let _ = playback::open(app, &self.settings, &self.runtime.theme, None);
            self.last_warm = Instant::now();
        }
        if self.seen.len() > 4096 {
            self.seen.retain(|process, eligible| {
                eligible.is_some_and(|start| start.elapsed() < Duration::from_secs(15))
                    || self.processes.contains(process)
                    || self.windows.values().any(|owner| owner == process)
            });
        }
    }
}

pub fn run(app: AppHandle, receiver: Receiver<Message>, sender: Sender<Message>) {
    let settings = app.state::<Host>().settings.lock().unwrap().clone();
    let mut monitor = Monitor::new(settings);
    let mut tick_at = Instant::now();
    let stopped = loop {
        match receiver.recv_timeout(Duration::from_millis(150)) {
            Ok(Message::Stop(reply)) => break reply,
            Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            Ok(Message::SettingsChanged) => monitor.settings_changed(&app),
            Ok(Message::SyncSettings(reply)) => {
                monitor.settings_changed(&app);
                let _ = reply.send(());
            }
            Ok(Message::WorkChanged) => monitor.animation_changed(&app),
            Ok(Message::Owned(window)) => {
                monitor.owned = window;
                if let Some(helper) = &mut monitor.helper {
                    helper.owned(monitor.owned.as_deref());
                }
            }
            Ok(Message::Launch(reply)) => {
                monitor.launch.push(reply);
                if monitor.runtime.connected {
                    monitor.complete_launch();
                } else if monitor.helper.is_none() {
                    monitor.connect_at = Instant::now();
                }
            }
            Ok(Message::Native(generation, event)) if monitor.generation == generation => {
                monitor.handle_event(&app, event);
            }
            Ok(Message::Closed(generation)) if monitor.generation == generation => {
                monitor.disconnect(&app, Some("Window listener disconnected".into()));
                monitor.complete_launch();
            }
            _ => {}
        }
        if Instant::now() >= tick_at {
            monitor.tick(&app, &sender);
            tick_at = Instant::now() + Duration::from_millis(150);
        }
    };
    drop(monitor);
    if let Some(reply) = stopped {
        let _ = reply.send(());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn windows_helper_process_event_needs_no_window_fields() {
        let event: WindowEvent = serde_json::from_value(serde_json::json!({
            "type": "process",
            "eventName": "BASELINE",
            "baseline": true,
            "pid": 9272,
            "executablePath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex\\app\\ChatGPT.exe",
            "processStartFileTime": "134382838018165681",
            "processStartedAt": "2026-11-04T16:36:41.8165681Z"
        }))
        .expect("Windows helper process JSON must deserialize without window fields");

        assert_eq!(event.process().as_deref(), Some("9272:134382838018165681"));
        assert!(event.hwnd.is_empty());
        assert!(event.baseline);
    }

    #[test]
    fn windows_helper_window_event_reaches_playback_geometry() {
        let event: WindowEvent = serde_json::from_value(serde_json::json!({
            "type": "window",
            "eventName": "SHOW",
            "baseline": false,
            "hwnd": "0x20942",
            "pid": 9272,
            "executablePath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex\\app\\ChatGPT.exe",
            "processStartFileTime": "134382838018165681",
            "processStartedAt": "2026-11-04T16:36:41.8165681Z",
            "dwmFrameBoundsPhysical": {"x": 137, "y": 130, "left": 137, "top": 130, "right": 937, "bottom": 730, "width": 800, "height": 600},
            "windowRectPhysical": {"x": 130, "y": 130, "left": 130, "top": 130, "right": 944, "bottom": 737, "width": 814, "height": 607},
            "rect": {"x": 137, "y": 130, "left": 137, "top": 130, "right": 937, "bottom": 730, "width": 800, "height": 600},
            "dpi": 168,
            "maximized": false,
            "minimized": false,
            "visible": true,
            "state": "normal",
            "cloaked": false,
            "foreground": true,
            "overlayEligible": true,
            "clipRects": [{"x": 0, "y": 0, "left": 0, "top": 0, "right": 800, "bottom": 600, "width": 800, "height": 600}]
        }))
        .expect("Windows helper JSON must deserialize as a window event");

        assert_eq!(event.process().as_deref(), Some("9272:134382838018165681"));
        assert!(!event.cloaked);
        let geometry = event
            .geometry()
            .expect("eligible window must reach playback");
        assert_eq!(
            geometry.bounds,
            Bounds {
                x: 137,
                y: 130,
                width: 800,
                height: 600
            }
        );
        assert!(geometry.presentable);
    }
}
