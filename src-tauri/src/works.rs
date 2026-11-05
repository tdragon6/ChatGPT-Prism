use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

pub(crate) const MAX_FILES: usize = 100;
pub(crate) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_WORK_BYTES: usize = 32 * 1024 * 1024;
const MAX_RENDER_BYTES: usize = 48 * 1024 * 1024;
const DEFAULT_AUTHOR: &str = "匿名";

static CHANGES: Mutex<()> = Mutex::new(());

const WORK_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; media-src data:; font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'";

const BOOTSTRAP_BEFORE: &str = r#"(()=>{
let initialized = false;
function setContext(c) {
  const root = document.documentElement;
  root.dataset.theme = c.theme;
  root.style.colorScheme = c.theme;
  c.reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
  window.__PRISM_CONTEXT__ = c;
  for (const media of document.querySelectorAll('video, audio')) {
    if (typeof c.parameters.sound === 'boolean') media.muted = !c.parameters.sound;
    if (typeof c.parameters.volume === 'number') media.volume = c.parameters.volume;
  }
}
addEventListener('message', event => {
  if (event.source !== parent) return;
  const data = event.data;
  if (data?.type === 'prism:init' && !initialized) {
    initialized = true;
    document.getElementById('prism-warmup')?.remove();
    const c = data.context;
    setContext(c);
    for (const media of document.querySelectorAll('video, audio')) {
      if (media.hasAttribute('data-prism-autoplay')) {
        media.removeAttribute('data-prism-autoplay');
        media.autoplay = true;
        media.play().catch(() => {});
      }
    }
"#;

const BOOTSTRAP_AFTER: &str = r#"
    window.dispatchEvent(new MessageEvent('message', { data, source:parent }));
  } else if (data?.type === 'prism:update' && initialized) {
    setContext(data.context);
  }
});
parent.postMessage({type:'prism:ready'}, '*');
})();"#;

fn default_author() -> String {
    DEFAULT_AUTHOR.into()
}

fn default_languages() -> Vec<String> {
    vec!["zh".into()]
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Work {
    pub id: String,
    pub name: String,
    #[serde(default = "default_author")]
    pub author: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub themes: Vec<String>,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    i18n: BTreeMap<String, Translation>,
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(rename = "type", skip_serializing)]
    kind: String,
    #[serde(skip_serializing)]
    api_version: u32,
    #[serde(skip_serializing)]
    entry: String,
    #[serde(default, skip_serializing)]
    locales: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    preview: Option<String>,
    #[serde(default, rename = "default", skip_serializing)]
    default_selection: bool,
    #[serde(default, skip_serializing)]
    legacy_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Translation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    parameters: BTreeMap<String, ParameterTranslation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParameterTranslation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    key: String,
    label: String,
    #[serde(rename = "type")]
    kind: ParameterKind,
    default: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    step: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<ParameterOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pattern: Option<TextPattern>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ParameterKind {
    Number,
    Boolean,
    Select,
    Text,
    Color,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum TextPattern {
    Label,
    Identity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ParameterOption {
    value: String,
    label: String,
}

#[derive(Clone, Serialize)]
pub struct Playback {
    pub html: String,
    pub context: Value,
    #[serde(skip)]
    pub themes: Vec<String>,
}

#[derive(Default)]
pub(crate) struct WorkFiles {
    files: BTreeMap<String, Vec<u8>>,
    lower_paths: HashSet<String>,
    bytes: usize,
}

impl WorkFiles {
    pub(crate) fn add(&mut self, relative: String, bytes: Vec<u8>) -> Result<(), String> {
        if !safe_path(&relative) || !self.lower_paths.insert(relative.to_ascii_lowercase()) {
            return Err(format!("Invalid or duplicate file path: {relative}"));
        }
        self.bytes += bytes.len();
        if self.files.len() >= MAX_FILES
            || bytes.len() > MAX_FILE_BYTES
            || self.bytes > MAX_WORK_BYTES
        {
            return Err(
                "An animation can contain up to 100 files, 8 MiB per file, 32 MiB total.".into(),
            );
        }
        self.files.insert(relative, bytes);
        Ok(())
    }

    fn get(&self, relative: &str) -> Result<&[u8], String> {
        self.files
            .get(relative)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("Missing animation file: {relative}"))
    }

    fn text(&self, relative: &str) -> Result<&str, String> {
        std::str::from_utf8(self.get(relative)?)
            .map_err(|_| format!("Animation file must use UTF-8: {relative}"))
    }

    fn asset(&self, relative: &str) -> Result<String, String> {
        let mime = match Path::new(relative).extension().and_then(|ext| ext.to_str()) {
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("mp3") => "audio/mpeg",
            Some("wav") => "audio/wav",
            Some("ogg") => "audio/ogg",
            Some("m4a") => "audio/mp4",
            Some("mp4") => "video/mp4",
            Some("webm") => "video/webm",
            _ => return Err(format!("Unsupported animation asset: {relative}")),
        };
        Ok(format!(
            "data:{mime};base64,{}",
            BASE64.encode(self.get(relative)?)
        ))
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn safe_segment(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or("").to_ascii_lowercase();
    !segment.starts_with('.')
        && !segment.ends_with('.')
        && !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        && !matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
        && !(base.len() == 4
            && matches!(&base[..3], "com" | "lpt")
            && (b'1'..=b'9').contains(&base.as_bytes()[3]))
}

pub(crate) fn safe_path(value: &str) -> bool {
    value.len() <= 240 && value.split('/').all(safe_segment)
}

fn resolve_relative(entry: &str, value: &str) -> Result<String, String> {
    let invalid = || format!("Invalid animation resource path: {value}");
    let (directory, _) = entry.rsplit_once('/').unwrap_or(("", entry));
    let mut path: Vec<&str> = directory
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in value.split('/') {
        match part {
            "." => (),
            ".." => {
                path.pop().ok_or_else(invalid)?;
            }
            part if safe_segment(part) => path.push(part),
            _ => return Err(invalid()),
        }
    }
    let relative = path.join("/");
    if !safe_path(&relative) {
        return Err(invalid());
    }
    Ok(relative)
}

fn safe_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Symbolic links are not supported in animations.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Reparse points are not supported in animations.".into());
        }
    }
    Ok(metadata)
}

fn load_local(path: &Path) -> Result<WorkFiles, String> {
    if !safe_metadata(path)?.is_dir() {
        return Err("Choose an animation folder.".into());
    }
    fn visit(root: &Path, folder: &Path, work: &mut WorkFiles) -> Result<(), String> {
        for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_str()
                .ok_or("Animation paths must use ASCII file names.")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !safe_path(&relative) {
                return Err(format!("Invalid animation file path: {relative}"));
            }
            let metadata = safe_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, work)?;
            } else if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES as u64 {
                let mut bytes = Vec::new();
                fs::File::open(&path)
                    .map_err(|error| error.to_string())?
                    .take((MAX_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                work.add(relative, bytes)?;
            } else {
                return Err(format!("Invalid or oversized animation file: {relative}"));
            }
        }
        Ok(())
    }
    let mut work = WorkFiles::default();
    visit(path, path, &mut work)?;
    Ok(work)
}

fn short_text(text: &str, limit: usize) -> bool {
    text.encode_utf16().count() <= limit
        && !text
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>'))
}

fn valid_label(label: &str, limit: usize, punctuation: &str) -> bool {
    !label.is_empty()
        && label.trim() == label
        && label.encode_utf16().count() <= limit
        && label
            .chars()
            .all(|character| character.is_alphanumeric() || punctuation.contains(character))
}

fn valid_choices(values: &[String], allowed: &[&str]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| allowed.contains(&value.as_str()))
        && values.iter().collect::<HashSet<_>>().len() == values.len()
}

fn valid_version(value: &str) -> bool {
    let components: Vec<_> = value.split('.').collect();
    components.len() == 3
        && components.iter().all(|component| {
            (1..=4).contains(&component.len())
                && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub(crate) fn newer(candidate: &str, current: &str) -> bool {
    let parts = |value: &str| {
        value
            .split('.')
            .map(str::parse::<u16>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    };
    matches!((parts(candidate), parts(current)), (Some(candidate), Some(current)) if candidate > current)
}

fn validate_manifest(mut work: Work) -> Result<Work, String> {
    if work.author.is_empty() {
        work.author = default_author();
    }
    if work.kind != "startup"
        || work.api_version != 1
        || !valid_id(&work.id)
        || !valid_label(&work.name, 60, " ._()·-")
        || !valid_label(&work.author, 40, " ._()·-")
        || !valid_version(&work.version)
        || !short_text(&work.description, 240)
        || !valid_choices(&work.themes, &["dark", "light"])
        || !valid_choices(&work.languages, &["zh", "en"])
        || work.parameters.len() > 24
        || work.legacy_ids.len() > 32
        || work
            .legacy_ids
            .iter()
            .any(|id| !valid_id(id) || id == &work.id)
        || work.legacy_ids.iter().collect::<HashSet<_>>().len() != work.legacy_ids.len()
    {
        return Err("Invalid startup animation metadata.".into());
    }
    for entry in std::iter::once(&work.entry).chain(work.locales.values()) {
        if !safe_path(entry) || !entry.ends_with(".html") {
            return Err(format!("Invalid animation entry point: {entry}"));
        }
    }
    if work
        .locales
        .keys()
        .chain(work.i18n.keys())
        .any(|locale| !work.languages.contains(locale))
    {
        return Err("Invalid animation locale.".into());
    }
    let mut parameter_keys = HashSet::new();
    for parameter in &work.parameters {
        if !parameter_keys.insert(&parameter.key) || !parameter.valid() {
            return Err(format!("Invalid animation parameter: {}", parameter.key));
        }
    }
    for translation in work.i18n.values() {
        if translation
            .name
            .as_ref()
            .is_some_and(|value| !valid_label(value, 60, " ._()·-"))
            || translation
                .description
                .as_ref()
                .is_some_and(|value| !short_text(value, 240))
            || translation.parameters.iter().any(|(key, translation)| {
                !parameter_keys.contains(key)
                    || translation
                        .label
                        .as_ref()
                        .is_some_and(|value| !valid_label(value, 40, " ._()·-"))
                    || translation
                        .description
                        .as_ref()
                        .is_some_and(|value| !short_text(value, 160))
            })
        {
            return Err("Invalid animation translation.".into());
        }
    }
    if let Some(preview) = work.preview.as_deref() {
        if !safe_path(preview)
            || !matches!(
                Path::new(preview).extension().and_then(|ext| ext.to_str()),
                Some("png" | "webp" | "jpg" | "jpeg")
            )
        {
            return Err("Invalid animation cover path.".into());
        }
    }
    Ok(work)
}

impl Work {
    pub(crate) fn preview_path(&self) -> Option<&str> {
        self.preview.as_deref()
    }

    fn required_files(&self) -> impl Iterator<Item = &str> {
        std::iter::once("meta.json")
            .chain(std::iter::once(self.entry.as_str()))
            .chain(self.locales.values().map(String::as_str))
            .chain(self.preview.as_deref())
    }
}

fn manifest(files: &WorkFiles, cover: bool) -> Result<Work, String> {
    let mut work = validate_manifest(
        serde_json::from_str(files.text("meta.json")?)
            .map_err(|error| format!("Invalid animation manifest: {error}"))?,
    )?;
    for required in work.required_files() {
        files.get(required)?;
    }
    if let Some(preview) = work.preview.as_deref().filter(|_| cover) {
        work.preview_url = Some(files.asset(preview)?);
    }
    Ok(work)
}

pub(crate) fn indexed_work(value: Value, paths: &HashSet<String>) -> Result<Work, String> {
    let work = validate_manifest(
        serde_json::from_value(value)
            .map_err(|error| format!("Invalid animation manifest: {error}"))?,
    )?;
    for required in work.required_files() {
        if !paths.contains(required) {
            return Err(format!("Missing animation file: {required}"));
        }
    }
    Ok(work)
}

impl Parameter {
    fn valid(&self) -> bool {
        if self.key.is_empty()
            || self.key.len() > 40
            || !self.key.as_bytes()[0].is_ascii_lowercase()
            || !self
                .key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || matches!(self.key.as_str(), "constructor" | "prototype" | "__proto__")
            || !valid_label(&self.label, 40, " ._()·-")
            || self
                .description
                .as_ref()
                .is_some_and(|description| !short_text(description, 160))
        {
            return false;
        }
        let common = match self.kind {
            ParameterKind::Number => {
                matches!((self.min, self.max, self.step), (Some(min), Some(max), Some(step))
                    if min.is_finite() && max.is_finite() && step > 0.0 && step <= max - min)
            }
            ParameterKind::Select => {
                (1..=24).contains(&self.options.len())
                    && self
                        .options
                        .iter()
                        .map(|option| &option.value)
                        .collect::<HashSet<_>>()
                        .len()
                        == self.options.len()
                    && self.options.iter().all(|option| {
                        (1..=40).contains(&option.value.len())
                            && option
                                .value
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                            && valid_label(&option.label, 40, " ._()·-")
                    })
            }
            ParameterKind::Text => self
                .max_length
                .is_some_and(|limit| (1..=120).contains(&limit)),
            ParameterKind::Boolean | ParameterKind::Color => true,
        };
        let known = match self.key.as_str() {
            "speed" => self.kind == ParameterKind::Number && self.min.is_some_and(|min| min > 0.0),
            "identity" => {
                self.kind == ParameterKind::Text
                    && self.pattern == Some(TextPattern::Identity)
                    && self.max_length.is_some_and(|limit| limit <= 32)
            }
            "sound" => self.kind == ParameterKind::Boolean,
            "volume" => {
                self.kind == ParameterKind::Number
                    && self.min.is_some_and(|min| min >= 0.0)
                    && self.max.is_some_and(|max| max <= 1.0)
            }
            _ => true,
        };
        common && known && self.accepts(&self.default)
    }

    fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            ParameterKind::Number => matches!(
                (value.as_f64(), self.min, self.max),
                (Some(value), Some(min), Some(max)) if value.is_finite() && (min..=max).contains(&value)
            ),
            ParameterKind::Boolean => value.is_boolean(),
            ParameterKind::Select => value
                .as_str()
                .is_some_and(|text| self.options.iter().any(|option| option.value == text)),
            ParameterKind::Text => value.as_str().is_some_and(|text| {
                let limit = self.max_length.unwrap_or(0);
                short_text(text, limit)
                    && match self.pattern {
                        Some(TextPattern::Label) => valid_label(text, limit, " ._()·-"),
                        Some(TextPattern::Identity) => valid_label(text, limit.min(32), " ._-"),
                        None => true,
                    }
            }),
            ParameterKind::Color => value.as_str().is_some_and(|text| {
                text.len() == 7
                    && text.starts_with('#')
                    && text[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
        }
    }
}

pub fn validate_parameters(work: &Work, values: &Value) -> Result<Value, String> {
    let values = values
        .as_object()
        .ok_or("Animation parameters must be an object.")?;
    if values.keys().any(|key| {
        !work
            .parameters
            .iter()
            .any(|parameter| parameter.key == *key)
    }) {
        return Err("Unknown animation parameter.".into());
    }
    let mut validated = Map::new();
    for parameter in &work.parameters {
        let value = values
            .get(&parameter.key)
            .cloned()
            .unwrap_or_else(|| parameter.default.clone());
        if !parameter.accepts(&value) {
            return Err(format!("Invalid value for {}.", parameter.label));
        }
        validated.insert(parameter.key.clone(), value);
    }
    Ok(Value::Object(validated))
}

struct Patterns {
    document: Regex,
    head_end: Regex,
    body_end: Regex,
    script: Regex,
    style: Regex,
    link: Regex,
    remove: Regex,
    source: Regex,
    href: Regex,
    rel: Regex,
    script_type: Regex,
    html_asset: Regex,
    media: Regex,
    autoplay: Regex,
    css_asset: Regex,
    css_import: Regex,
    script_end: Regex,
    style_end: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let regex = |pattern| Regex::new(pattern).expect("valid work pattern");
    Patterns {
        document: regex(
            r"(?is)^\s*(?:<!doctype\s+html\s*>\s*)?<html(?:\s[^>]*)?>\s*<head(?:\s[^>]*)?>",
        ),
        head_end: regex(r"(?i)</head\s*>"),
        body_end: regex(r"(?i)</body\s*>"),
        script: regex(r"(?is)<script\b([^>]*)>(.*?)</script\s*>"),
        style: regex(r"(?is)<style\b[^>]*>(.*?)</style\s*>"),
        link: regex(r"(?is)<link\b[^>]*>"),
        remove: regex(r"(?is)<base\b[^>]*>|<meta\b[^>]*http-equiv[^>]*>"),
        source: regex(r#"(?i)\bsrc\s*=\s*["']([^"']+)["']"#),
        href: regex(r#"(?i)\bhref\s*=\s*["']([^"']+)["']"#),
        rel: regex(r#"(?i)\brel\s*=\s*["']stylesheet["']"#),
        script_type: regex(r#"(?i)\btype\s*=\s*["']([^"']+)["']"#),
        html_asset: regex(r#"(?i)\b(src|poster)\s*=\s*["']([^"']+)["']"#),
        media: regex(r"(?is)<(?:audio|video)\b[^>]*>"),
        autoplay: regex(r#"(?i)\sautoplay(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?([\s>]|/>)"#),
        css_asset: regex(r#"(?i)url\(\s*["']?([^)'"\s]+)["']?\s*\)"#),
        css_import: regex(r"(?i)@import\b"),
        script_end: regex(r"(?i)</script"),
        style_end: regex(r"(?i)</style"),
    }
});

fn replace_resources(
    source: &str,
    pattern: &Regex,
    mut replace: impl FnMut(Captures<'_>) -> Result<String, String>,
) -> Result<String, String> {
    let mut result = String::with_capacity(source.len());
    let mut offset = 0;
    for captures in pattern.captures_iter(source) {
        let matched = captures.get(0).expect("match");
        append(&mut result, &source[offset..matched.start()])?;
        append(&mut result, &replace(captures)?)?;
        offset = matched.end();
    }
    append(&mut result, &source[offset..])?;
    Ok(result)
}

fn append(target: &mut String, value: &str) -> Result<(), String> {
    if target.len() + value.len() > MAX_RENDER_BYTES {
        return Err("Prepared animation is too large.".into());
    }
    target.push_str(value);
    Ok(())
}

fn embedded_asset(files: &WorkFiles, relative_to: &str, value: &str) -> Result<String, String> {
    if value.starts_with("data:") {
        return Ok(value.into());
    }
    files.asset(&resolve_relative(relative_to, value)?)
}

fn stylesheet(files: &WorkFiles, relative_to: &str, css: &str) -> Result<String, String> {
    if PATTERNS.css_import.is_match(css) {
        return Err("Load local stylesheets with HTML links. CSS imports are unsupported.".into());
    }
    replace_resources(css, &PATTERNS.css_asset, |capture| {
        let resource = &capture[1];
        if resource.starts_with('#') {
            return Ok(capture[0].into());
        }
        Ok(format!(
            "url(\"{}\")",
            embedded_asset(files, relative_to, resource)?
        ))
    })
}

fn animation_html(files: &WorkFiles, entry: &str) -> Result<String, String> {
    let r = &*PATTERNS;
    let html = files.text(entry)?.trim_start_matches('\u{feff}');
    if !r.document.is_match(html) || !r.head_end.is_match(html) || !r.body_end.is_match(html) {
        return Err("Provide a complete animation HTML document with head and body.".into());
    }
    let mut javascript = String::new();
    for captures in r.script.captures_iter(html) {
        let attributes = &captures[1];
        if let Some(kind) = r.script_type.captures(attributes) {
            if !matches!(&kind[1], "text/javascript" | "application/javascript") {
                return Err("Use local classic scripts in startup animations.".into());
            }
        }
        javascript.push_str("\n;");
        if let Some(source) = r.source.captures(attributes) {
            append(
                &mut javascript,
                files.text(&resolve_relative(entry, &source[1])?)?,
            )?;
        } else {
            append(&mut javascript, &captures[2])?;
        }
    }
    let html = r.script.replace_all(html, "");
    let mut css = String::new();
    for item in r.link.find_iter(&html) {
        if r.rel.is_match(item.as_str()) {
            let link = r
                .href
                .captures(item.as_str())
                .ok_or("Missing stylesheet path.")?;
            css.push('\n');
            let source = resolve_relative(entry, &link[1])?;
            append(&mut css, &stylesheet(files, &source, files.text(&source)?)?)?;
        }
    }
    let html = r.link.replace_all(&html, "");
    for captures in r.style.captures_iter(&html) {
        css.push('\n');
        append(&mut css, &stylesheet(files, entry, &captures[1])?)?;
    }
    let html = r.style.replace_all(&html, "");
    let html = r.remove.replace_all(&html, "");
    let html = replace_resources(&html, &r.html_asset, |capture| {
        Ok(format!(
            "{}=\"{}\"",
            &capture[1],
            embedded_asset(files, entry, &capture[2])?
        ))
    })?;
    let mut html = r
        .media
        .replace_all(&html, |tag: &Captures<'_>| {
            r.autoplay
                .replace_all(&tag[0], " data-prism-autoplay${1}")
                .into_owned()
        })
        .into_owned();
    let end = r
        .document
        .find(&html)
        .ok_or("Invalid animation document.")?
        .end();
    html.insert_str(
        end,
        &format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{WORK_CSP}\">"),
    );
    let css = r.style_end.replace_all(&css, "<\\/style");
    let style = format!(
        "<style>{css}</style><style id=\"prism-warmup\">*,*::before,*::after{{animation-play-state:paused!important}}</style>"
    );
    let end = r
        .head_end
        .find(&html)
        .ok_or("Missing animation head.")?
        .start();
    html.insert_str(end, &style);

    let mut script = String::from(BOOTSTRAP_BEFORE);
    if !javascript.contains(".speed")
        && !javascript.contains("[\"speed\"]")
        && !javascript.contains("['speed']")
    {
        script.push_str(
            "    if (typeof c.parameters.speed === 'number') {\n      document.documentElement.style.setProperty('--prism-speed', String(c.parameters.speed));\n      for (const media of document.querySelectorAll('video, audio')) media.playbackRate = c.parameters.speed;\n    }\n",
        );
    }
    script.push_str(&javascript);
    script.push_str("\n;");
    script.push_str(BOOTSTRAP_AFTER);
    let script = format!(
        "<script>{}</script>",
        r.script_end.replace_all(&script, "<\\/script")
    );
    let end = r
        .body_end
        .find(&html)
        .ok_or("Missing animation body.")?
        .start();
    if html.len() + script.len() > MAX_RENDER_BYTES {
        return Err("Prepared animation is too large.".into());
    }
    html.insert_str(end, &script);
    Ok(html)
}

fn library_root(data_dir: &Path) -> PathBuf {
    data_dir.join("startup")
}

fn installed_metadata(path: &Path, id: &str) -> Result<Work, String> {
    let work = manifest(&load_local(path)?, false)?;
    if work.id != id {
        return Err("Animation folder and manifest IDs differ.".into());
    }
    Ok(work)
}

fn exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn recover_update(root: &Path, id: &str) -> Result<(), String> {
    let backup = root.join(format!(".update-{id}"));
    if !exists(&backup)? {
        return Ok(());
    }
    let destination = root.join(id);
    if exists(&destination)? {
        installed_metadata(&destination, id)?;
        if installed_metadata(&backup, id).is_ok() {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(())
    } else {
        installed_metadata(&backup, id)?;
        fs::rename(backup, destination).map_err(|error| error.to_string())
    }
}

fn work_files(data_dir: &Path, id: &str) -> Result<WorkFiles, String> {
    if !valid_id(id) {
        return Err("Invalid animation ID.".into());
    }
    let library = library_root(data_dir);
    if !safe_metadata(&library)?.is_dir() {
        return Err("Animation library is not a directory.".into());
    }
    recover_update(&library, id)?;
    load_local(&library.join(id))
}

pub fn list(data_dir: &Path) -> Result<Vec<Work>, String> {
    let _lock = CHANGES.lock().map_err(|error| error.to_string())?;
    let mut works = Vec::new();
    let library = library_root(data_dir);
    match fs::symlink_metadata(&library) {
        Ok(_) => {
            if !safe_metadata(&library)?.is_dir() {
                return Err("Animation library is not a directory.".into());
            }
            for entry in fs::read_dir(&library).map_err(|error| error.to_string())? {
                let name = entry.map_err(|error| error.to_string())?.file_name();
                if let Some(id) = name.to_str().and_then(|name| name.strip_prefix(".update-")) {
                    if valid_id(id) {
                        let _ = recover_update(&library, id);
                    }
                }
            }
            for entry in fs::read_dir(&library).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                let Some(id) = path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                if !valid_id(id) {
                    continue;
                }
                if let Ok(work) = load_local(&path).and_then(|files| manifest(&files, true)) {
                    if work.id == id {
                        works.push(work);
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    works.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(works)
}

#[derive(Default)]
pub struct PackagedWorks {
    pub default_id: Option<String>,
    pub aliases: BTreeMap<String, String>,
}

impl PackagedWorks {
    fn archive_root(data_dir: &Path) -> Result<PathBuf, String> {
        let root = data_dir.join("startup-archive");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        if !safe_metadata(&root)?.is_dir() {
            return Err("Animation archive is not a directory.".into());
        }
        Ok(root)
    }

    fn archive_current(data_dir: &Path, id: &str) -> Result<PathBuf, String> {
        fn directories(
            source: &Path,
            folder: &Path,
            found: &mut Vec<String>,
        ) -> Result<(), String> {
            for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if safe_metadata(&path)?.is_dir() {
                    found.push(
                        path.strip_prefix(source)
                            .map_err(|error| error.to_string())?
                            .to_str()
                            .ok_or("Invalid animation archive path.")?
                            .replace(std::path::MAIN_SEPARATOR, "/"),
                    );
                    directories(source, &path, found)?;
                }
            }
            Ok(())
        }
        fn fingerprint(files: &WorkFiles, directories: &[String]) -> String {
            let mut digest = Sha256::new();
            for directory in directories {
                digest.update(b"D");
                digest.update((directory.len() as u64).to_be_bytes());
                digest.update(directory.as_bytes());
            }
            for (path, bytes) in &files.files {
                digest.update(b"F");
                digest.update((path.len() as u64).to_be_bytes());
                digest.update(path.as_bytes());
                digest.update((bytes.len() as u64).to_be_bytes());
                digest.update(bytes);
            }
            format!("{:x}", digest.finalize())
        }
        let source = library_root(data_dir).join(id);
        let files = load_local(&source)?;
        let mut folders = Vec::new();
        directories(&source, &source, &mut folders)?;
        folders.sort();
        let digest = fingerprint(&files, &folders);
        let root = Self::archive_root(data_dir)?;
        let archive = root.join(format!("{id}-{digest}"));
        if exists(&archive)? {
            if !safe_metadata(&archive)?.is_dir() {
                return Err("Animation archive is not a directory.".into());
            }
            let stored = archive.join(id);
            let stored_files = load_local(&stored)?;
            let mut stored_folders = Vec::new();
            directories(&stored, &stored, &mut stored_folders)?;
            stored_folders.sort();
            if fingerprint(&stored_files, &stored_folders) != digest {
                return Err("The previous animation archive has changed.".into());
            }
            return Ok(archive);
        }
        let stage = tempfile::Builder::new()
            .prefix(".snapshot-")
            .tempdir_in(root)
            .map_err(|error| error.to_string())?;
        let destination = stage.path().join(id);
        fs::create_dir(&destination).map_err(|error| error.to_string())?;
        for folder in &folders {
            fs::create_dir_all(destination.join(folder)).map_err(|error| error.to_string())?;
        }
        for (relative, bytes) in files.files {
            let target = destination.join(relative);
            fs::create_dir_all(target.parent().ok_or("Invalid animation archive path.")?)
                .map_err(|error| error.to_string())?;
            fs::write(target, bytes).map_err(|error| error.to_string())?;
        }
        fs::rename(stage.path(), &archive).map_err(|error| error.to_string())?;
        Ok(archive)
    }

    pub fn preserve_settings(&self, data_dir: &Path) -> Result<(), String> {
        let source = data_dir.join("settings.json");
        if !exists(&source)? {
            return Ok(());
        }
        let root = Self::archive_root(data_dir)?;
        let mut backup = tempfile::Builder::new()
            .prefix("settings-before-migration-")
            .suffix(".json")
            .tempfile_in(root)
            .map_err(|error| error.to_string())?;
        let mut source = fs::File::open(source).map_err(|error| error.to_string())?;
        std::io::copy(&mut source, &mut backup).map_err(|error| error.to_string())?;
        backup
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        backup.keep().map_err(|error| error.to_string())?;
        Ok(())
    }

    // Call only after the remapped settings have been saved. A failed or
    // interrupted move leaves either the old library or a recoverable archive.
    pub fn archive_legacy(&self, data_dir: &Path) -> Result<(), String> {
        let _lock = CHANGES.lock().map_err(|error| error.to_string())?;
        let library = library_root(data_dir);
        for (legacy, current) in &self.aliases {
            let source = library.join(legacy);
            if !exists(&source)? || !exists(&library.join(current))? {
                continue;
            }
            if !safe_metadata(&source)?.is_dir() {
                return Err("Legacy animation is not a directory.".into());
            }
            let root = Self::archive_root(data_dir)?;
            let archive = tempfile::Builder::new()
                .prefix(&format!("{legacy}-"))
                .tempdir_in(root)
                .map_err(|error| error.to_string())?
                .keep();
            if let Err(error) = fs::rename(&source, archive.join(legacy)) {
                let _ = fs::remove_dir(&archive);
                return Err(error.to_string());
            }
        }
        Ok(())
    }
}

pub fn seed_packaged(data_dir: &Path, packaged: &Path) -> Result<PackagedWorks, String> {
    let marker = data_dir.join(".bundled-startup-v1");
    let seeded = exists(&marker)?;
    let library = library_root(data_dir);
    let shared_library = packaged == library;
    let entries = match fs::read_dir(packaged) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PackagedWorks::default());
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut candidates = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        if !safe_metadata(&path)?.is_dir() {
            continue;
        }
        let metadata = (|| {
            let file = path.join("meta.json");
            let info = safe_metadata(&file)?;
            if !info.is_file() || info.len() > MAX_FILE_BYTES as u64 {
                return Err("Invalid packaged animation metadata.".into());
            }
            let work = validate_manifest(
                serde_json::from_slice::<Work>(&fs::read(file).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?,
            )?;
            if work.id != id {
                return Err("Animation folder and manifest IDs differ.".into());
            }
            Ok(work)
        })();
        match metadata {
            Ok(work) => {
                candidates.insert(work.id.clone(), (path, work));
            }
            // On Windows the package resources share the editable library.
            // Invalid user imports must not prevent other works from loading.
            Err(_) if shared_library => (),
            Err(error) => return Err(error),
        }
    }
    // In a shared editable library, obsolete manifests may still declare their
    // own defaults and aliases. Only the final replacement's configuration is
    // authoritative; it lists every supported historical ID explicitly.
    let replaced_ids = candidates
        .values()
        .flat_map(|(_, work)| work.legacy_ids.iter().cloned())
        .collect::<HashSet<_>>();
    candidates.retain(|id, _| !replaced_ids.contains(id));
    if candidates.is_empty() && !replaced_ids.is_empty() {
        return Err("Packaged animation aliases cannot form cycles.".into());
    }
    let mut plan = PackagedWorks::default();
    for (_, work) in candidates.values() {
        for alias in &work.legacy_ids {
            if plan
                .aliases
                .insert(alias.clone(), work.id.clone())
                .is_some()
            {
                return Err("Packaged animation aliases must be unique.".into());
            }
        }
    }
    let defaults = candidates
        .values()
        .filter(|(_, work)| work.default_selection)
        .map(|(_, work)| work.id.clone())
        .collect::<Vec<_>>();
    if defaults.len() > 1 && !shared_library {
        return Err("Only one packaged animation may be the default.".into());
    }
    plan.default_id = defaults.into_iter().next();
    for (id, (path, work)) in &candidates {
        let has_legacy = work.legacy_ids.iter().try_fold(false, |found, alias| {
            Ok::<_, String>(found || exists(&library.join(alias))?)
        })?;
        recover_update(&library, id)?;
        if exists(&library.join(id))? {
            if !shared_library {
                let current = installed_metadata(&library.join(id), id)?;
                if newer(&work.version, &current.version) {
                    let files = load_local(path)?;
                    let replacement = manifest(&files, true)?;
                    for entry in
                        std::iter::once(&replacement.entry).chain(replacement.locales.values())
                    {
                        animation_html(&files, entry)?;
                    }
                    PackagedWorks::archive_current(data_dir, id)?;
                    install_files(data_dir, files, replacement, true)?;
                }
            }
            continue;
        }
        if seeded && !has_legacy {
            continue;
        }
        import_work(data_dir, path)?;
    }
    let installed_ids = list(data_dir)?
        .into_iter()
        .map(|work| work.id)
        .collect::<HashSet<_>>();
    plan.aliases.retain(|_, id| installed_ids.contains(id));
    if plan
        .default_id
        .as_ref()
        .is_some_and(|id| !installed_ids.contains(id))
    {
        plan.default_id = None;
    }
    if !seeded && !candidates.is_empty() {
        fs::write(marker, b"1").map_err(|error| error.to_string())?;
    }
    Ok(plan)
}

pub fn prepare(
    data_dir: &Path,
    id: &str,
    locale: &str,
    theme: &str,
    parameters: &Value,
    preview: bool,
) -> Result<Playback, String> {
    let files = {
        let _lock = CHANGES.lock().map_err(|error| error.to_string())?;
        work_files(data_dir, id)?
    };
    let work = manifest(&files, false)?;
    if work.id != id {
        return Err("Animation folder and manifest IDs differ.".into());
    }
    prepare_files(&files, &work, locale, theme, parameters, preview)
}

pub(crate) fn prepare_catalog_files(
    files: &WorkFiles,
    expected: &Work,
    locale: &str,
    theme: &str,
    parameters: &Value,
) -> Result<Playback, String> {
    let work = verified_catalog_work(files, expected)?;
    prepare_files(files, &work, locale, theme, parameters, true)
}

fn prepare_files(
    files: &WorkFiles,
    work: &Work,
    locale: &str,
    theme: &str,
    parameters: &Value,
    preview: bool,
) -> Result<Playback, String> {
    let locale = if work.languages.iter().any(|language| language == locale) {
        locale
    } else {
        &work.languages[0]
    };
    let theme = if work.themes.iter().any(|supported| supported == theme) {
        theme
    } else if preview {
        &work.themes[0]
    } else {
        return Err("The animation does not support the current ChatGPT theme.".into());
    };
    let html = animation_html(files, work.locales.get(locale).unwrap_or(&work.entry))?;
    Ok(Playback {
        html,
        themes: work.themes.clone(),
        context: json!({
            "theme": theme,
            "locale": locale,
            "parameters": validate_parameters(work, parameters)?,
            "preview": preview,
            "reducedMotion": false,
            "hostControls": { "skip": !preview },
        }),
    })
}

pub fn import_work(data_dir: &Path, folder: &Path) -> Result<Work, String> {
    let files = load_local(folder)?;
    let work = manifest(&files, true)?;
    if folder.file_name().and_then(|name| name.to_str()) != Some(work.id.as_str()) {
        return Err("Animation folder and manifest IDs differ.".into());
    }
    install_files(data_dir, files, work, false)
}

pub(crate) fn install_from_catalog(
    data_dir: &Path,
    files: WorkFiles,
    expected: &Work,
) -> Result<(), String> {
    let work = verified_catalog_work(&files, expected)?;
    install_files(data_dir, files, work, true).map(|_| ())
}

fn verified_catalog_work(files: &WorkFiles, expected: &Work) -> Result<Work, String> {
    let work = manifest(files, false)?;
    if &work != expected {
        return Err("Downloaded animation metadata differs from the marketplace index.".into());
    }
    Ok(work)
}

fn install_files(
    data_dir: &Path,
    files: WorkFiles,
    work: Work,
    allow_update: bool,
) -> Result<Work, String> {
    for entry in std::iter::once(&work.entry).chain(work.locales.values()) {
        animation_html(&files, entry)?;
    }
    let _lock = CHANGES.lock().map_err(|error| error.to_string())?;
    let root = library_root(data_dir);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    if !safe_metadata(&root)?.is_dir() {
        return Err("Animation library is not a directory.".into());
    }
    recover_update(&root, &work.id)?;
    let destination = root.join(&work.id);
    let replacing = exists(&destination)?;
    if replacing
        && (!allow_update
            || !newer(
                &work.version,
                &installed_metadata(&destination, &work.id)?.version,
            ))
    {
        return Err("An animation with this ID is already installed.".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(&root)
        .map_err(|error| error.to_string())?;
    for (relative, bytes) in files.files {
        let target = stage.path().join(relative);
        fs::create_dir_all(target.parent().ok_or("Invalid animation target.")?)
            .map_err(|error| error.to_string())?;
        fs::write(&target, bytes).map_err(|error| error.to_string())?;
    }
    let backup = root.join(format!(".update-{}", work.id));
    if replacing {
        fs::rename(&destination, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(stage.path(), &destination) {
        if replacing {
            let _ = fs::rename(&backup, &destination);
        }
        return Err(error.to_string());
    }
    if replacing {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(work)
}

pub fn remove_work(
    data_dir: &Path,
    id: &str,
    save_selection: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _lock = CHANGES.lock().map_err(|error| error.to_string())?;
    if !valid_id(id) {
        return Err("This animation cannot be removed.".into());
    }
    let root = library_root(data_dir);
    if !safe_metadata(&root)?.is_dir() {
        return Err("Animation library is not a directory.".into());
    }
    recover_update(&root, id)?;
    if exists(&root.join(format!(".update-{id}")))? {
        return Err("The animation update backup could not be removed.".into());
    }
    let source = root.join(id);
    installed_metadata(&source, id)?;
    let removal = tempfile::Builder::new()
        .prefix(".remove-")
        .tempdir_in(&root)
        .map_err(|error| error.to_string())?;
    let staged = removal.path().join(id);
    fs::rename(&source, &staged).map_err(|error| error.to_string())?;
    if let Err(error) = save_selection() {
        return match fs::rename(&staged, &source) {
            Ok(()) => Err(error),
            Err(restore) => {
                let recovery = removal.keep();
                Err(format!(
                    "{error}. Could not restore the animation: {restore}. Files kept in {}",
                    recovery.display()
                ))
            }
        };
    }
    removal.close().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn sample_work(root: &Path, id: &str, default: bool, legacy: &[&str]) -> PathBuf {
        let folder = root.join(id);
        fs::create_dir_all(&folder).unwrap();
        let metadata = json!({
            "apiVersion": 1, "type": "startup", "id": id, "name": "Sample",
            "version": "1.0.0", "themes": ["dark", "light"], "languages": ["zh", "en"],
            "entry": "index.html", "default": default, "legacyIds": legacy,
            "parameters": [
                {"key":"speed","label":"Speed","type":"number","default":1,"min":0.5,"max":2,"step":0.25},
                {"key":"volume","label":"Volume","type":"number","default":0.5,"min":0,"max":1,"step":0.05}
            ]
        });
        fs::write(
            folder.join("meta.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        fs::write(folder.join("index.html"), "<!doctype html><html><head></head><body><script>window.sample = true;</script></body></html>").unwrap();
        folder
    }

    fn set_version(folder: &Path, version: &str) {
        let file = folder.join("meta.json");
        let mut metadata: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        metadata["version"] = json!(version);
        fs::write(file, serde_json::to_vec(&metadata).unwrap()).unwrap();
    }

    #[test]
    fn packaged_startup_is_seeded_once() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        sample_work(packaged.path(), "alpha-work", false, &[]);
        sample_work(packaged.path(), "zeta-work", true, &[]);
        let plan = seed_packaged(data.path(), packaged.path()).unwrap();
        let mut settings = Settings::default();
        settings.select_default_work(&list(data.path()).unwrap(), plan.default_id.as_deref());
        assert_eq!(settings.work_id, "zeta-work");
        assert!(settings.enabled);
        let ids = list(data.path())
            .unwrap()
            .into_iter()
            .map(|work| work.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["alpha-work", "zeta-work"]);

        fs::remove_dir_all(data.path().join("startup").join("zeta-work")).unwrap();
        let plan = seed_packaged(data.path(), packaged.path()).unwrap();
        assert!(plan.default_id.is_none());
        let ids = list(data.path())
            .unwrap()
            .into_iter()
            .map(|work| work.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["alpha-work"]);
    }

    #[test]
    fn legacy_only_install_upgrades_from_package_and_archives_after_saved_selection() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        sample_work(packaged.path(), "new-work", true, &["old-work"]);
        let old = sample_work(&data.path().join("startup"), "old-work", false, &[]);
        fs::write(old.join("custom.txt"), b"personal version").unwrap();
        fs::write(data.path().join(".bundled-startup-v1"), b"1").unwrap();
        let mut settings = Settings {
            work_id: "old-work".into(),
            parameters: json!({"speed":1.5,"volume":0.2}),
            ..Settings::default()
        };
        settings
            .work_parameters
            .insert("old-work".into(), json!({"volume":0.7}));
        settings.save(data.path()).unwrap();
        let original_settings = fs::read(data.path().join("settings.json")).unwrap();
        let plan = seed_packaged(data.path(), packaged.path()).unwrap();
        assert!(old.exists());
        assert_eq!(
            fs::read(data.path().join("startup/new-work/index.html")).unwrap(),
            fs::read(packaged.path().join("new-work/index.html")).unwrap()
        );
        assert!(settings.migrate_work_ids(&plan.aliases));
        assert_eq!(settings.work_id, "new-work");
        assert_eq!(settings.parameters, json!({"speed":1.5,"volume":0.2}));
        assert!(settings.enabled);
        plan.preserve_settings(data.path()).unwrap();
        settings.save(data.path()).unwrap();
        plan.archive_legacy(data.path()).unwrap();
        assert!(!old.exists());
        let archive = data.path().join("startup-archive");
        let entries = fs::read_dir(&archive)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert!(
            entries
                .iter()
                .any(|path| path.is_file() && fs::read(path).unwrap() == original_settings)
        );
        assert!(
            entries
                .iter()
                .any(|path| path.join("old-work/custom.txt").is_file())
        );
        let second = seed_packaged(data.path(), packaged.path()).unwrap();
        let mut reloaded = Settings::load(data.path());
        assert!(!reloaded.migrate_work_ids(&second.aliases));
        second.archive_legacy(data.path()).unwrap();
        assert_eq!(fs::read_dir(&archive).unwrap().count(), entries.len());
        assert_eq!(list(data.path()).unwrap().len(), 1);
    }

    #[test]
    fn existing_canonical_content_wins_and_interrupted_migration_is_retryable() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        sample_work(packaged.path(), "new-work", true, &["old-work"]);
        sample_work(&data.path().join("startup"), "old-work", false, &[]);
        let canonical = sample_work(&data.path().join("startup"), "new-work", false, &[]);
        fs::write(canonical.join("custom.txt"), b"edited new version").unwrap();
        fs::write(data.path().join(".bundled-startup-v1"), b"1").unwrap();
        seed_packaged(data.path(), packaged.path()).unwrap();
        // Simulate interruption after installing the replacement but before saving settings.
        let plan = seed_packaged(data.path(), packaged.path()).unwrap();
        let mut settings = Settings {
            work_id: "old-work".into(),
            enabled: false,
            parameters: json!({"speed":0.75}),
            ..Settings::default()
        };
        settings
            .work_parameters
            .insert("new-work".into(), json!({"volume":0.4,"speed":2}));
        settings.migrate_work_ids(&plan.aliases);
        assert_eq!(settings.parameters, json!({"speed":0.75,"volume":0.4}));
        assert!(!settings.enabled);
        settings.save(data.path()).unwrap();
        plan.archive_legacy(data.path()).unwrap();
        assert_eq!(
            fs::read(canonical.join("custom.txt")).unwrap(),
            b"edited new version"
        );
        assert_eq!(
            list(data.path())
                .unwrap()
                .iter()
                .map(|work| &work.id)
                .collect::<Vec<_>>(),
            ["new-work"]
        );
    }

    #[test]
    fn newer_packaged_version_updates_canonical_work_with_complete_recoverable_snapshot() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        let replacement = sample_work(packaged.path(), "sample-work", true, &[]);
        set_version(&replacement, "2.0.0");
        let current = sample_work(&data.path().join("startup"), "sample-work", true, &[]);
        fs::write(current.join("custom.txt"), b"personal content").unwrap();
        fs::create_dir_all(current.join("empty/nested")).unwrap();
        let original_files = load_local(&current).unwrap().files;
        fs::write(data.path().join(".bundled-startup-v1"), b"1").unwrap();
        seed_packaged(data.path(), packaged.path()).unwrap();
        assert_eq!(list(data.path()).unwrap()[0].version, "2.0.0");
        assert!(!current.join("custom.txt").exists());
        let archive = data.path().join("startup-archive");
        let snapshots = fs::read_dir(&archive)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 1);
        let stored = snapshots[0].join("sample-work");
        assert_eq!(load_local(&stored).unwrap().files, original_files);
        assert!(stored.join("empty/nested").is_dir());
        seed_packaged(data.path(), packaged.path()).unwrap();
        assert_eq!(fs::read_dir(&archive).unwrap().count(), 1);
    }

    #[test]
    fn equal_or_newer_local_versions_keep_custom_content() {
        for local_version in ["2.0.0", "3.0.0"] {
            let data = tempfile::tempdir().unwrap();
            let packaged = tempfile::tempdir().unwrap();
            let replacement = sample_work(packaged.path(), "sample-work", true, &[]);
            set_version(&replacement, "2.0.0");
            let current = sample_work(&data.path().join("startup"), "sample-work", true, &[]);
            set_version(&current, local_version);
            fs::write(current.join("custom.txt"), b"retain this edit").unwrap();
            seed_packaged(data.path(), packaged.path()).unwrap();
            assert_eq!(list(data.path()).unwrap()[0].version, local_version);
            assert_eq!(
                fs::read(current.join("custom.txt")).unwrap(),
                b"retain this edit"
            );
            assert!(!data.path().join("startup-archive").exists());
        }
    }

    #[test]
    fn invalid_packaged_upgrade_preserves_current_work_without_partial_archives() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        let replacement = sample_work(packaged.path(), "sample-work", true, &[]);
        set_version(&replacement, "2.0.0");
        fs::write(
            replacement.join("index.html"),
            "invalid replacement document",
        )
        .unwrap();
        let current = sample_work(&data.path().join("startup"), "sample-work", true, &[]);
        let original_files = load_local(&current).unwrap().files;
        for _ in 0..2 {
            assert!(seed_packaged(data.path(), packaged.path()).is_err());
            assert_eq!(load_local(&current).unwrap().files, original_files);
            assert!(!data.path().join("startup-archive").exists());
        }
    }

    #[test]
    fn interrupted_packaged_upgrade_reuses_the_completed_snapshot() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        let replacement = sample_work(packaged.path(), "sample-work", true, &[]);
        set_version(&replacement, "2.0.0");
        sample_work(&data.path().join("startup"), "sample-work", true, &[]);
        // Simulate interruption after publishing the snapshot, before install_files.
        let first = PackagedWorks::archive_current(data.path(), "sample-work").unwrap();
        assert_eq!(
            PackagedWorks::archive_current(data.path(), "sample-work").unwrap(),
            first
        );
        seed_packaged(data.path(), packaged.path()).unwrap();
        assert_eq!(
            fs::read_dir(data.path().join("startup-archive"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(list(data.path()).unwrap()[0].version, "2.0.0");
    }

    #[test]
    fn windows_shared_library_ignores_legacy_default_and_preserves_other_preferences() {
        let data = tempfile::tempdir().unwrap();
        let library = data.path().join("startup");
        sample_work(&library, "old-work", true, &[]);
        sample_work(&library, "new-work", true, &["old-work"]);
        let plan = seed_packaged(data.path(), &library).unwrap();
        assert_eq!(plan.default_id.as_deref(), Some("new-work"));
        let mut settings = Settings {
            work_id: "other-work".into(),
            parameters: json!({}),
            ..Settings::default()
        };
        settings
            .work_parameters
            .insert("old-work".into(), json!({"speed":1.25,"volume":0.3}));
        settings
            .work_parameters
            .insert("new-work".into(), json!({"volume":0.8}));
        settings.migrate_work_ids(&plan.aliases);
        assert_eq!(settings.work_id, "other-work");
        assert_eq!(
            settings.work_parameters["new-work"],
            json!({"speed":1.25,"volume":0.8})
        );
        settings.save(data.path()).unwrap();
        plan.archive_legacy(data.path()).unwrap();
        assert!(!library.join("old-work").exists());
        assert!(library.join("new-work").exists());
    }

    #[test]
    fn conflicting_package_defaults_fail_before_importing_any_work() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        sample_work(packaged.path(), "alpha-work", true, &[]);
        sample_work(packaged.path(), "zeta-work", true, &[]);
        assert!(seed_packaged(data.path(), packaged.path()).is_err());
        assert!(list(data.path()).unwrap().is_empty());
    }

    #[test]
    fn windows_shared_library_allows_imported_defaults_without_changing_selection() {
        let data = tempfile::tempdir().unwrap();
        let library = data.path().join("startup");
        sample_work(&library, "bundled-work", true, &[]);
        sample_work(&library, "user-import", true, &[]);
        fs::write(data.path().join(".bundled-startup-v1"), b"1").unwrap();
        let plan = seed_packaged(data.path(), &library).unwrap();
        assert_eq!(plan.default_id.as_deref(), Some("bundled-work"));
        let mut settings = Settings {
            work_id: "user-import".into(),
            parameters: json!({"volume":0.6}),
            ..Settings::default()
        };
        settings.migrate_work_ids(&plan.aliases);
        settings.select_default_work(&list(data.path()).unwrap(), plan.default_id.as_deref());
        assert_eq!(settings.work_id, "user-import");
        assert_eq!(settings.parameters, json!({"volume":0.6}));
        assert_eq!(list(data.path()).unwrap().len(), 2);
    }

    #[test]
    fn replacement_manifest_supersedes_legacy_alias_configuration() {
        let data = tempfile::tempdir().unwrap();
        let library = data.path().join("startup");
        sample_work(&library, "ancient-work", false, &[]);
        sample_work(&library, "old-work", true, &["ancient-work"]);
        sample_work(&library, "new-work", true, &["old-work", "ancient-work"]);
        let plan = seed_packaged(data.path(), &library).unwrap();
        assert_eq!(plan.default_id.as_deref(), Some("new-work"));
        assert_eq!(plan.aliases.len(), 2);
        assert_eq!(plan.aliases["ancient-work"], "new-work");
        assert_eq!(plan.aliases["old-work"], "new-work");
        let mut settings = Settings {
            work_id: "ancient-work".into(),
            parameters: json!({"volume":0.3}),
            ..Settings::default()
        };
        settings.migrate_work_ids(&plan.aliases);
        settings.save(data.path()).unwrap();
        plan.archive_legacy(data.path()).unwrap();
        assert_eq!(settings.work_id, "new-work");
        assert_eq!(settings.parameters, json!({"volume":0.3}));
        assert_eq!(list(data.path()).unwrap().len(), 1);
        seed_packaged(data.path(), &library).unwrap();
    }

    #[test]
    fn cyclic_package_aliases_are_rejected_without_importing_works() {
        let data = tempfile::tempdir().unwrap();
        let packaged = tempfile::tempdir().unwrap();
        sample_work(packaged.path(), "alpha-work", true, &["zeta-work"]);
        sample_work(packaged.path(), "zeta-work", false, &["alpha-work"]);
        assert!(seed_packaged(data.path(), packaged.path()).is_err());
        assert!(list(data.path()).unwrap().is_empty());
    }

    #[test]
    fn media_settings_update_without_restarting_autoplay() {
        let before = r#"
const assert = (condition, message) => { if (!condition) throw new Error(message); };
let listener;
let playCount = 0;
let autoplayPending = true;
const media = {
  muted: false, volume: 0.5,
  hasAttribute: () => autoplayPending,
  removeAttribute: () => { autoplayPending = false; },
  play: () => { playCount++; return Promise.resolve(); }
};
globalThis.window = globalThis;
globalThis.parent = { postMessage() {} };
globalThis.document = {
  documentElement: { dataset: {}, style: {} },
  getElementById: () => ({ remove() {} }),
  querySelectorAll: () => [media]
};
globalThis.matchMedia = () => ({ matches: false });
globalThis.addEventListener = (_, callback) => { listener = callback; };
globalThis.dispatchEvent = (event) => listener(event);
globalThis.MessageEvent = class { constructor(_, init) { Object.assign(this, init); } };
"#;
        let after = r#"
const send = (type, sound, volume) => listener({source:parent, data:{type, context:{theme:'dark', parameters:{sound, volume}}}});
send('prism:init', true, 0.2);
assert(!media.muted && media.volume === 0.2 && playCount === 1, 'Initial audio settings were not applied');
send('prism:update', false, 0.7);
assert(media.muted && media.volume === 0.7 && playCount === 1, 'Mute/volume update must apply without replaying media');
send('prism:update', true, 0.4);
assert(!media.muted && media.volume === 0.4 && playCount === 1, 'Unmute must preserve the active playback');
"#;
        let script = [before, BOOTSTRAP_BEFORE, BOOTSTRAP_AFTER, after].concat();
        let output = std::process::Command::new("node")
            .args(["-e", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn authored_parameter_defaults_are_preserved_in_lists_and_playback() {
        let data = tempfile::tempdir().unwrap();
        let folder = sample_work(&data.path().join("startup"), "sample-work", true, &[]);
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(folder.join("meta.json")).unwrap()).unwrap();
        metadata["parameters"][0]["default"] = json!(1.25);
        metadata["parameters"][1]["default"] = json!(0.2);
        fs::write(
            folder.join("meta.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let work = list(data.path()).unwrap().remove(0);
        assert_eq!(work.parameters[0].default, json!(1.25));
        assert_eq!(work.parameters[1].default, json!(0.2));
        let playback = prepare(data.path(), "sample-work", "en", "dark", &json!({}), true).unwrap();
        assert_eq!(
            playback.context["parameters"],
            json!({"speed":1.25,"volume":0.2})
        );
    }
}
