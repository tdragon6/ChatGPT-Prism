use crate::works::{self, Playback, Work, WorkFiles};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use ureq::{
    Agent,
    tls::{RootCerts, TlsConfig, TlsProvider},
};
use url::Url;

pub const PROJECT_URL: &str = "https://github.com/tdragon6/ChatGPT-Prism";
pub const DEFAULT_MARKET_URL: &str =
    "https://raw.githubusercontent.com/tdragon6/ChatGPT-Prism/main/market-index.json";

const VERSION_FILE: &str = "https://raw.githubusercontent.com/tdragon6/ChatGPT-Prism/main/version";
const INDEX_LIMIT: usize = 2 * 1024 * 1024;
const INDEX_NAME: &str = "market-index.json";
static INDEX_WRITES: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    work: Work,
    installed_version: Option<String>,
    update_available: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub latest_version: String,
    pub available: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    schema_version: u32,
    items: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    manifest: Value,
    source_url: String,
    files: Vec<InventoryFile>,
    sha256: String,
    size: usize,
}

#[derive(Deserialize, Serialize)]
struct InventoryFile {
    path: String,
    size: usize,
    sha256: String,
}

struct AuditedItem {
    work: Work,
    source: Url,
    preview_url: Option<String>,
    files: Vec<InventoryFile>,
    inventory_digest: String,
}

#[derive(PartialEq, Eq)]
struct PreviewKey {
    id: String,
    index: Url,
    source: Url,
    inventory_digest: String,
}

struct PreviewCache {
    entry: Option<(PreviewKey, Arc<WorkFiles>)>,
    generation: u64,
}

static PREVIEW_CACHE: Mutex<PreviewCache> = Mutex::new(PreviewCache {
    entry: None,
    generation: 0,
});

pub fn evict_preview(id: &str) {
    let mut cache = PREVIEW_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if cache.entry.as_ref().is_some_and(|(key, _)| key.id == id) {
        cache.entry = None;
    }
    // An in-flight request must not put a deleted work back in the cache.
    cache.generation = cache.generation.wrapping_add(1);
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn web_url(value: &str, require_https: bool) -> Result<Url, String> {
    let scheme = if require_https {
        "HTTPS"
    } else {
        "HTTP or HTTPS"
    };
    if value.len() > 2048 || value.chars().any(|value| value.is_control()) {
        return Err(format!("Invalid {scheme} address."));
    }
    let url = Url::parse(value).map_err(|_| format!("Invalid {scheme} address."))?;
    if !(url.scheme() == "https" || (!require_https && url.scheme() == "http"))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(format!(
            "Use an {scheme} address without credentials, query parameters or fragments."
        ));
    }
    Ok(url)
}

fn index_url(source: &str) -> Result<Url, String> {
    let mut url = web_url(
        if source.trim().is_empty() {
            DEFAULT_MARKET_URL
        } else {
            source.trim()
        },
        true,
    )?;
    if !url.path().ends_with(".json") {
        url.path_segments_mut()
            .map_err(|_| "Invalid HTTPS marketplace address.")?
            .pop_if_empty()
            .push(INDEX_NAME);
    }
    Ok(url)
}

pub fn validate_source(source: &str) -> Result<(), String> {
    index_url(source).map(|_| ())
}

fn agent(require_https: bool) -> Agent {
    Agent::config_builder()
        .user_agent("ChatGPT-Prism")
        .https_only(require_https)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(15)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("Marketplace response exceeds the size limit.".into());
    }
    Ok(bytes)
}

fn fetch(agent: &Agent, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = agent
        .get(url)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|error| error.to_string())?;
    if response.status().as_u16() != 200 {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return Err("Marketplace response exceeds the size limit.".into());
    }
    read_limited(response.body_mut().as_reader(), limit)
}

fn inventory_order(a: &str, b: &str) -> std::cmp::Ordering {
    // Match the index inventory's ASCII path order before hashing.
    let weight = |byte: u8| match byte {
        b'_' => 0,
        b'-' => 1,
        b'.' => 2,
        b'/' => 3,
        byte => byte.to_ascii_lowercase(),
    };
    a.bytes()
        .map(weight)
        .cmp(b.bytes().map(weight))
        .then_with(|| {
            a.bytes()
                .map(|byte| byte.is_ascii_uppercase())
                .cmp(b.bytes().map(|byte| byte.is_ascii_uppercase()))
        })
}

fn audit_item(index: &Url, raw: Value) -> Result<AuditedItem, String> {
    let item: Item = serde_json::from_value(raw).map_err(|error| error.to_string())?;
    if item.files.is_empty() || item.files.len() > works::MAX_FILES || !valid_hash(&item.sha256) {
        return Err("Invalid animation file inventory.".into());
    }
    let mut lowercase_paths = HashSet::new();
    let mut paths = HashSet::new();
    let mut total_size = 0;
    for file in &item.files {
        if !works::safe_path(&file.path)
            || !lowercase_paths.insert(file.path.to_ascii_lowercase())
            || !valid_hash(&file.sha256)
            || file.size > works::MAX_FILE_BYTES
        {
            return Err("Invalid or duplicate animation file.".into());
        }
        total_size += file.size;
        paths.insert(file.path.clone());
    }
    if total_size == 0 || total_size != item.size || total_size > works::MAX_WORK_BYTES {
        return Err("Invalid animation size.".into());
    }
    let mut ordered = item.files.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| inventory_order(&a.path, &b.path));
    let digest = hash(&serde_json::to_vec(&ordered).map_err(|error| error.to_string())?);
    if digest != item.sha256 {
        return Err("Invalid animation inventory checksum.".into());
    }
    let work = works::indexed_work(item.manifest, &paths)?;
    if item.source_url.trim().is_empty() {
        return Err("Missing animation source address.".into());
    }
    let mut source = web_url(
        index
            .join(&item.source_url)
            .map_err(|error| error.to_string())?
            .as_str(),
        false,
    )?;
    if !source.path().ends_with('/') {
        source
            .path_segments_mut()
            .map_err(|_| "Invalid animation source address.")?
            .push("");
    }
    let preview_url = work
        .preview_path()
        .map(|path| source.join(path).map(|url| url.to_string()))
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(AuditedItem {
        work,
        source,
        preview_url,
        files: item.files,
        inventory_digest: digest,
    })
}

fn parse_index(index: &Url, bytes: &[u8]) -> Result<BTreeMap<String, AuditedItem>, String> {
    let parsed: Index = serde_json::from_slice(bytes)
        .map_err(|_| "The marketplace did not return a valid JSON index.")?;
    if parsed.schema_version != 1 || parsed.items.len() > 1000 {
        return Err("Unsupported marketplace index.".into());
    }
    let mut audited = BTreeMap::new();
    for raw in parsed.items {
        match raw.pointer("/manifest/type").and_then(Value::as_str) {
            Some("startup") => {
                let item = audit_item(index, raw)?;
                if audited.insert(item.work.id.clone(), item).is_some() {
                    return Err("Duplicate animation ID in marketplace.".into());
                }
            }
            Some(_) => (),
            None => return Err("Invalid marketplace work type.".into()),
        }
    }
    Ok(audited)
}

fn source_folder(index: &Url) -> String {
    hash(index.as_str().as_bytes())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    false
}

fn directory(path: &Path, create: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => Ok(()),
        Ok(_) => Err("Invalid marketplace index directory.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if create {
                if let Err(error) = fs::create_dir(path) {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(error.to_string());
                    }
                    directory(path, false)?;
                }
            }
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn index_path(data_dir: &Path, index: &Url, create: bool) -> Result<PathBuf, String> {
    directory(data_dir, create)?;
    let cache = data_dir.join("market-cache");
    directory(&cache, create)?;
    let folder = cache.join(source_folder(index));
    directory(&folder, create)?;
    Ok(folder.join(INDEX_NAME))
}

fn read_index(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file() && !is_link(&metadata) && metadata.len() <= limit as u64 =>
        {
            let file = fs::File::open(path).map_err(|error| error.to_string())?;
            read_limited(file, limit).map(Some)
        }
        Ok(_) => Err("Invalid marketplace index file.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn store(data_dir: &Path, index: &Url, bytes: &[u8]) -> Result<(), String> {
    let _write = INDEX_WRITES.lock().map_err(|error| error.to_string())?;
    let path = index_path(data_dir, index, true)?;
    store_at(&path, bytes)
}

fn store_at(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid marketplace index path.")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    staged
        .write_all(bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    staged.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}

fn local_index(data_dir: &Path, index: &Url) -> Result<BTreeMap<String, AuditedItem>, String> {
    let path = index_path(data_dir, index, false)?;
    match read_index(&path, INDEX_LIMIT)? {
        Some(bytes) => parse_index(index, &bytes),
        None => Ok(BTreeMap::new()),
    }
}

fn invalidate_index(data_dir: &Path, index: &Url) -> Result<(), String> {
    let _write = INDEX_WRITES.lock().map_err(|error| error.to_string())?;
    let path = index_path(data_dir, index, false)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) => {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        Ok(_) => return Err("Invalid marketplace index file.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    let mut preview = PREVIEW_CACHE.lock().map_err(|error| error.to_string())?;
    if preview
        .entry
        .as_ref()
        .is_some_and(|(key, _)| key.index == *index)
    {
        preview.entry = None;
    }
    preview.generation = preview.generation.wrapping_add(1);
    Ok(())
}

fn refresh_index(
    data_dir: &Path,
    index: &Url,
    client: &Agent,
) -> Result<BTreeMap<String, AuditedItem>, String> {
    let bytes = match fetch(client, index.as_str(), INDEX_LIMIT) {
        Ok(bytes) => bytes,
        Err(error) => {
            if matches!(error.as_str(), "HTTP 404" | "HTTP 410") {
                invalidate_index(data_dir, index)?;
            }
            return Err(error);
        }
    };
    let remote = parse_index(index, &bytes)?;
    store(data_dir, index, &bytes)?;
    Ok(remote)
}

fn catalog(remote: BTreeMap<String, AuditedItem>, installed: Vec<Work>) -> Catalog {
    let installed_by_id: HashMap<_, _> = installed
        .iter()
        .map(|work| (work.id.as_str(), work))
        .collect();
    let entries = remote
        .into_values()
        .map(|item| {
            let mut work = item.work;
            work.preview_url = item.preview_url;
            let installed = installed_by_id.get(work.id.as_str());
            CatalogEntry {
                update_available: installed
                    .is_some_and(|installed| works::newer(&work.version, &installed.version)),
                installed_version: installed.map(|installed| installed.version.clone()),
                work,
            }
        })
        .collect();

    Catalog { entries }
}

pub fn get(data_dir: &Path, source: &str) -> Result<Catalog, String> {
    refresh(data_dir, source)
}

pub fn refresh(data_dir: &Path, source: &str) -> Result<Catalog, String> {
    let index = index_url(source)?;
    let installed = works::list(data_dir)?;
    let remote = refresh_index(data_dir, &index, &agent(true))?;
    Ok(catalog(remote, installed))
}

pub fn install(data_dir: &Path, source: &str, id: &str) -> Result<(), String> {
    let index = index_url(source)?;
    let item = market_work(data_dir, &index, id)?;
    if let Some(installed) = works::list(data_dir)?
        .into_iter()
        .find(|work| work.id == id)
    {
        if !works::newer(&item.work.version, &installed.version) {
            return Err("An animation with this ID is already installed.".into());
        }
    }
    let files = download_work(&item)?;
    works::install_from_catalog(data_dir, files, &item.work)
}

fn market_work(data_dir: &Path, index: &Url, id: &str) -> Result<AuditedItem, String> {
    if !works::valid_id(id) {
        return Err("Invalid animation ID.".into());
    }
    local_index(data_dir, index)?
        .remove(id)
        .ok_or_else(|| "Animation not found in the marketplace.".into())
}

fn download_work(item: &AuditedItem) -> Result<WorkFiles, String> {
    let agent = agent(false);
    let started = Instant::now();
    let mut files = WorkFiles::default();
    for record in &item.files {
        if started.elapsed() > Duration::from_secs(90) {
            return Err("The animation download timed out.".into());
        }
        let url = item
            .source
            .join(&record.path)
            .map_err(|error| error.to_string())?;
        let bytes = fetch(&agent, url.as_str(), record.size)?;
        if bytes.len() != record.size || hash(&bytes) != record.sha256 {
            return Err(format!("Animation file checksum failed: {}", record.path));
        }
        files.add(record.path.clone(), bytes)?;
    }
    Ok(files)
}

pub fn preview(
    data_dir: &Path,
    source: &str,
    id: &str,
    locale: &str,
    theme: &str,
    parameters: &Value,
) -> Result<Playback, String> {
    let index = index_url(source)?;
    let item = market_work(data_dir, &index, id)?;
    works::validate_parameters(&item.work, parameters)?;
    let key = PreviewKey {
        id: id.to_owned(),
        index: index.clone(),
        source: item.source.clone(),
        inventory_digest: item.inventory_digest.clone(),
    };
    let (cached, generation) = {
        let cache = PREVIEW_CACHE.lock().map_err(|error| error.to_string())?;
        let cached = cache
            .entry
            .as_ref()
            .filter(|(stored, _)| *stored == key)
            .map(|(_, files)| Arc::clone(files));
        (cached, cache.generation)
    };
    let new_files = cached.is_none();
    let files = match cached {
        Some(files) => files,
        None => Arc::new(download_work(&item)?),
    };
    let prepared = works::prepare_catalog_files(&files, &item.work, locale, theme, parameters)?;
    let latest = market_work(data_dir, &index, id)?;
    if latest.work != item.work
        || latest.source != item.source
        || latest.inventory_digest != item.inventory_digest
    {
        return Err("The marketplace animation changed. Open its preview again.".into());
    }
    if new_files {
        let mut cache = PREVIEW_CACHE.lock().map_err(|error| error.to_string())?;
        if cache.generation == generation {
            cache.entry = Some((key, files));
        }
    }
    Ok(prepared)
}

fn version(text: &str) -> Result<Version, String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    let parsed = Version::parse(text.strip_prefix('v').unwrap_or(text))
        .map_err(|_| "The release server did not return a valid version.")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("The release server did not return a stable version.".into());
    }
    Ok(parsed)
}

pub fn release_url(release: &str) -> Result<String, String> {
    Ok(format!("{PROJECT_URL}/releases/tag/v{}", version(release)?))
}

pub fn check_app_update(current: &str) -> Result<Update, String> {
    let current = version(current)?;
    let bytes = fetch(&agent(true), VERSION_FILE, 128)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "The release server did not return a valid version.")?;
    let latest = version(text)?;
    let latest_version = latest.to_string();
    Ok(Update {
        latest_version,
        available: latest.cmp_precedence(&current).is_gt(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;

    fn sample_files() -> (Value, BTreeMap<String, Vec<u8>>) {
        let manifest = json!({
            "apiVersion": 1, "type": "startup", "id": "sample-work", "name": "Sample",
            "version": "1.0.0", "themes": ["dark", "light"], "languages": ["en"],
            "entry": "index.html", "preview": "cover.png"
        });
        let files = BTreeMap::from([
            ("meta.json".into(), serde_json::to_vec(&manifest).unwrap()),
            ("cover.png".into(), b"sample cover".to_vec()),
            (
                "index.html".into(),
                b"<!doctype html><html><head></head><body>Verified HTTP work</body></html>"
                    .to_vec(),
            ),
        ]);
        (manifest, files)
    }

    fn sample_item(source: &str, manifest: &Value, contents: &BTreeMap<String, Vec<u8>>) -> Value {
        let mut files = contents
            .iter()
            .map(|(path, bytes)| InventoryFile {
                path: path.clone(),
                size: bytes.len(),
                sha256: hash(bytes),
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| inventory_order(&a.path, &b.path));
        json!({
            "manifest": manifest, "sourceUrl": source,
            "size": files.iter().map(|file| file.size).sum::<usize>(),
            "sha256": hash(&serde_json::to_vec(&files).unwrap()), "files": files
        })
    }

    fn http_files(
        contents: BTreeMap<String, Vec<u8>>,
        requests: usize,
    ) -> (String, std::thread::JoinHandle<()>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let address = format!(
            "http://{}/assets/sample-work/",
            server.local_addr().unwrap()
        );
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut served = 0;
            while served < requests {
                let (mut stream, _) = match server.accept() {
                    Ok(connection) => connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => {
                        panic!("HTTP test server did not receive the expected request: {error}")
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).unwrap();
                    assert!(read > 0 && request.len() + read <= 8192);
                    request.extend_from_slice(&buffer[..read]);
                }
                let request = std::str::from_utf8(&request).unwrap();
                let path = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let relative = path.strip_prefix("/assets/sample-work/").unwrap();
                let body = contents.get(relative).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
                served += 1;
            }
        });
        (address, worker)
    }

    fn http_index_response(status: u16, body: Vec<u8>) -> (Url, std::thread::JoinHandle<()>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let address = Url::parse(&format!(
            "http://{}/market-index.json",
            server.local_addr().unwrap()
        ))
        .unwrap();
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match server.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => {
                        panic!("HTTP index test server did not receive a request: {error}")
                    }
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0 && request.len() + read <= 8192);
                request.extend_from_slice(&buffer[..read]);
            }
            assert!(request.starts_with(b"GET /market-index.json HTTP/1.1\r\n"));
            write!(
                stream,
                "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (address, worker)
    }

    #[test]
    fn missing_remote_index_invalidates_only_its_cached_catalog_and_preview() {
        let temp = tempfile::tempdir().unwrap();
        let (manifest, files) = sample_files();
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "items": [sample_item("https://assets.example/sample-work/", &manifest, &files)],
        }))
        .unwrap();
        let other = index_url("https://another.example/market-index.json").unwrap();
        store(temp.path(), &other, &bytes).unwrap();
        for status in [404, 410] {
            let (index, server) = http_index_response(status, Vec::new());
            store(temp.path(), &index, &bytes).unwrap();
            assert_eq!(local_index(temp.path(), &index).unwrap().len(), 1);
            {
                let mut preview = PREVIEW_CACHE.lock().unwrap();
                preview.entry = Some((
                    PreviewKey {
                        id: "sample-work".into(),
                        index: index.clone(),
                        source: Url::parse("https://assets.example/sample-work/").unwrap(),
                        inventory_digest: String::new(),
                    },
                    Arc::new(WorkFiles::default()),
                ));
            }
            let error = match refresh_index(temp.path(), &index, &agent(false)) {
                Ok(_) => panic!("A missing remote index must not return cached works"),
                Err(error) => error,
            };
            server.join().unwrap();
            assert_eq!(error, format!("HTTP {status}"));
            assert!(local_index(temp.path(), &index).unwrap().is_empty());
            assert!(!index_path(temp.path(), &index, false).unwrap().exists());
            assert!(PREVIEW_CACHE.lock().unwrap().entry.is_none());
            assert_eq!(local_index(temp.path(), &other).unwrap().len(), 1);
        }
    }

    #[test]
    fn transient_remote_failure_is_an_error_instead_of_a_cached_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let (manifest, files) = sample_files();
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "items": [sample_item("https://assets.example/sample-work/", &manifest, &files)],
        }))
        .unwrap();
        let (index, server) = http_index_response(503, Vec::new());
        store(temp.path(), &index, &bytes).unwrap();
        let error = match refresh_index(temp.path(), &index, &agent(false)) {
            Ok(_) => panic!("A network failure must not be presented as a fresh catalog"),
            Err(error) => error,
        };
        server.join().unwrap();
        assert_eq!(error, "HTTP 503");
        assert_eq!(local_index(temp.path(), &index).unwrap().len(), 1);
    }

    #[test]
    fn all_sources_are_isolated_and_the_legacy_root_index_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let (manifest, files) = sample_files();
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "items": [sample_item("https://assets.example/sample-work/", &manifest, &files)],
        }))
        .unwrap();
        fs::write(temp.path().join(INDEX_NAME), &bytes).unwrap();
        let default = index_url(DEFAULT_MARKET_URL).unwrap();
        let other = index_url("https://another.example/market-index.json").unwrap();
        assert!(local_index(temp.path(), &default).unwrap().is_empty());
        assert!(local_index(temp.path(), &other).unwrap().is_empty());
        store(temp.path(), &default, &bytes).unwrap();
        assert_eq!(local_index(temp.path(), &default).unwrap().len(), 1);
        assert!(local_index(temp.path(), &other).unwrap().is_empty());
        assert_ne!(
            index_path(temp.path(), &default, false).unwrap(),
            temp.path().join(INDEX_NAME)
        );
        assert_eq!(fs::read(temp.path().join(INDEX_NAME)).unwrap(), bytes);
    }

    #[test]
    fn animation_sources_accept_relative_and_absolute_http_https_directories() {
        let index = index_url("https://index.example/market/index.json").unwrap();
        let (manifest, files) = sample_files();
        for (source, expected) in [
            (
                "startup/sample-work/",
                "https://index.example/market/startup/sample-work/",
            ),
            (
                "https://cdn.example/another-repo/sample-work",
                "https://cdn.example/another-repo/sample-work/",
            ),
            (
                "http://assets.example/sample-work/",
                "http://assets.example/sample-work/",
            ),
        ] {
            let item = audit_item(&index, sample_item(source, &manifest, &files)).unwrap();
            assert_eq!(item.source.as_str(), expected);
            assert_eq!(
                item.preview_url.as_deref(),
                Some(format!("{expected}cover.png").as_str())
            );
        }
    }

    #[test]
    fn source_addresses_and_inventory_paths_remain_validated() {
        let index = index_url("https://index.example/market/index.json").unwrap();
        let (manifest, files) = sample_files();
        for source in [
            "ftp://assets.example/work/",
            "file:///work/",
            "https://user:pass@assets.example/work/",
            "http://assets.example/work/?token=x",
            "https://assets.example/work/#fragment",
        ] {
            assert!(
                audit_item(&index, sample_item(source, &manifest, &files)).is_err(),
                "{source}"
            );
        }
        for path in [
            "../index.html",
            "https://assets.example/index.html",
            "/index.html",
        ] {
            let mut item = sample_item("http://assets.example/work/", &manifest, &files);
            let mut inventory =
                serde_json::from_value::<Vec<InventoryFile>>(item["files"].clone()).unwrap();
            inventory.push(InventoryFile {
                path: path.into(),
                size: 1,
                sha256: hash(b"x"),
            });
            inventory.sort_by(|a, b| inventory_order(&a.path, &b.path));
            item["size"] = json!(inventory.iter().map(|file| file.size).sum::<usize>());
            item["sha256"] = json!(hash(&serde_json::to_vec(&inventory).unwrap()));
            item["files"] = serde_json::to_value(inventory).unwrap();
            assert!(audit_item(&index, item).is_err(), "{path}");
        }
        let mut item = sample_item("http://assets.example/work/", &manifest, &files);
        item["sha256"] = json!("0".repeat(64));
        assert!(audit_item(&index, item).is_err());
        assert!(index_url("http://index.example/market/index.json").is_err());
        assert!(index_url("https://index.example/market/index.json").is_ok());
    }

    #[test]
    fn http_animation_downloads_verify_every_file_before_playback() {
        let index = index_url("https://index.example/market/index.json").unwrap();
        let (manifest, contents) = sample_files();
        let (source, worker) = http_files(contents.clone(), contents.len());
        let item = audit_item(&index, sample_item(&source, &manifest, &contents)).unwrap();
        let files = download_work(&item).unwrap();
        worker.join().unwrap();
        let playback =
            works::prepare_catalog_files(&files, &item.work, "en", "dark", &json!({})).unwrap();
        assert!(playback.html.contains("Verified HTTP work"));
        assert_eq!(playback.context["theme"], "dark");

        let mut altered = contents.clone();
        altered.get_mut("cover.png").unwrap()[0] ^= 1;
        let (source, worker) = http_files(altered, 1);
        let item = audit_item(&index, sample_item(&source, &manifest, &contents)).unwrap();
        let error = match download_work(&item) {
            Ok(_) => panic!("A changed HTTP file must fail its SHA-256 check"),
            Err(error) => error,
        };
        worker.join().unwrap();
        assert!(error.contains("checksum failed: cover.png"), "{error}");
    }

    #[test]
    fn missing_cached_index_is_empty_without_creating_a_file() {
        let temp = tempfile::tempdir().unwrap();
        for source in [DEFAULT_MARKET_URL, "https://example.com/market/index.json"] {
            let result = local_index(temp.path(), &index_url(source).unwrap()).unwrap();
            assert!(result.is_empty());
            assert!(
                !index_path(temp.path(), &index_url(source).unwrap(), false)
                    .unwrap()
                    .exists()
            );
        }
        assert!(!temp.path().join("market-cache").exists());
    }

    #[test]
    fn market_cover_uses_the_index_source_url() {
        let temp = tempfile::tempdir().unwrap();
        let packaged = Path::new(env!("CARGO_MANIFEST_DIR")).join("../startup");
        works::seed_packaged(temp.path(), &packaged).unwrap();
        let installed = works::list(temp.path()).unwrap();
        assert!(
            installed
                .iter()
                .find(|work| work.id == "key-protocol")
                .unwrap()
                .preview_url
                .as_deref()
                .is_some_and(|url| url.starts_with("data:"))
        );
        let index = index_url(DEFAULT_MARKET_URL).unwrap();
        let sample =
            fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../market-index.json")).unwrap();
        let items = parse_index(&index, &sample).unwrap();
        let result = catalog(items, installed);
        let cover = result
            .entries
            .iter()
            .find(|entry| entry.work.id == "key-protocol")
            .unwrap()
            .work
            .preview_url
            .as_deref();
        assert_eq!(
            cover,
            Some(
                "https://raw.githubusercontent.com/tdragon6/ChatGPT-Prism/main/startup/key-protocol/cover.png"
            )
        );
    }
}
