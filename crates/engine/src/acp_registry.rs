use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeron_proto::{AcpDistribution, AcpPreparedAgent, AcpRegistryAgent};

use crate::EngineError;
use crate::project_actions::persist_json;

const INDEX_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const ICON_HOST: &str = "cdn.agentclientprotocol.com";
const INDEX_MAX_AGE: Duration = Duration::from_secs(300);
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ICON_BYTES: usize = 64 * 1024;
const MAX_SEARCH_RESULTS: usize = 20;

#[derive(Debug, Clone, Deserialize)]
struct Agent {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    distribution: Distribution,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Distribution {
    #[serde(default)]
    binary: BTreeMap<String, Value>,
    #[serde(default)]
    npx: Option<Package>,
    #[serde(default)]
    uvx: Option<Package>,
}

#[derive(Debug, Clone, Deserialize)]
struct Package {
    package: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BinaryTarget {
    archive: String,
    cmd: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

impl Agent {
    fn binary_target(&self) -> Option<BinaryTarget> {
        let target = self.distribution.binary.get(platform_key()?)?;
        serde_json::from_value(target.clone()).ok()
    }

    fn distributions(&self) -> Vec<AcpDistribution> {
        [
            (AcpDistribution::Binary, self.binary_target().is_some()),
            (AcpDistribution::Npx, self.distribution.npx.is_some()),
            (AcpDistribution::Uvx, self.distribution.uvx.is_some()),
        ]
        .into_iter()
        .filter_map(|(distribution, offered)| offered.then_some(distribution))
        .collect()
    }
}

fn platform_key() -> Option<&'static str> {
    zeron_harness::acp::registry_platform()
        .filter(|key| key.starts_with("darwin-") || key.starts_with("linux-"))
}

/// Agent ids and versions name directories, so only plain path components pass.
fn path_safe(value: &str) -> bool {
    value.len() <= 128
        && value
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn parse_index(body: &str) -> Result<Vec<Agent>, String> {
    let index: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let agents = index
        .get("agents")
        .and_then(Value::as_array)
        .ok_or("the index has no agents list")?;
    Ok(agents
        .iter()
        .filter_map(
            |agent| match serde_json::from_value::<Agent>(agent.clone()) {
                Ok(agent)
                    if path_safe(&agent.id)
                        && path_safe(&agent.version)
                        && !agent.name.trim().is_empty() =>
                {
                    Some(agent)
                }
                Ok(agent) => {
                    tracing::warn!(
                        id = agent.id,
                        "skipping ACP Registry agent with unusable id, version or name"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "skipping malformed ACP Registry agent");
                    None
                }
            },
        )
        .collect())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedIndex {
    #[serde(default)]
    etag: Option<String>,
    fetched_at: u64,
    body: String,
}

fn registry_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("acp-registry")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn http() -> Result<reqwest::Client, EngineError> {
    reqwest::Client::builder()
        .user_agent(concat!("zeron/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| EngineError::Other(format!("ACP Registry client: {error}")))
}

enum Fetched {
    NotModified,
    Body { body: String, etag: Option<String> },
}

async fn fetch_index(url: &str, etag: Option<&str>) -> Result<Fetched, String> {
    let mut request = http().map_err(|error| error.to_string())?.get(url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if body.len() > MAX_INDEX_BYTES {
        return Err(format!("the index is larger than {MAX_INDEX_BYTES} bytes"));
    }
    let body = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    Ok(Fetched::Body { body, etag })
}

async fn load_index(data_dir: &Path, url: &str) -> Result<Vec<Agent>, EngineError> {
    let path = registry_dir(data_dir).join("index.json");
    let cached = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CachedIndex>(&bytes).ok());
    let now = now_secs();
    if let Some(cached) = &cached
        && now >= cached.fetched_at
        && now - cached.fetched_at < INDEX_MAX_AGE.as_secs()
        && let Ok(agents) = parse_index(&cached.body)
    {
        return Ok(agents);
    }
    let fetched = fetch_index(url, cached.as_ref().and_then(|c| c.etag.as_deref())).await;
    let fresh = match (fetched, cached) {
        (Ok(Fetched::NotModified), Some(mut cached)) => {
            cached.fetched_at = now;
            Ok(cached)
        }
        (Ok(Fetched::Body { body, etag }), cached) => match parse_index(&body) {
            Ok(_) => Ok(CachedIndex {
                etag,
                fetched_at: now,
                body,
            }),
            Err(error) => Err((
                format!("the ACP Registry answered an invalid index: {error}"),
                cached,
            )),
        },
        (Ok(Fetched::NotModified), None) => Err((
            "the ACP Registry answered 304 to an uncached request".into(),
            None,
        )),
        (Err(error), cached) => Err((format!("the ACP Registry is unreachable: {error}"), cached)),
    };
    match fresh {
        Ok(index) => {
            if let Err(error) = persist_json(&path, &index) {
                tracing::warn!(%error, "could not cache the ACP Registry index");
            }
            parse_index(&index.body).map_err(EngineError::Other)
        }
        Err((error, Some(cached))) => {
            tracing::warn!(error, "serving the cached ACP Registry index");
            parse_index(&cached.body).map_err(EngineError::Other)
        }
        Err((error, None)) => Err(EngineError::Other(error)),
    }
}

pub(crate) async fn search(
    data_dir: &Path,
    query: Option<&str>,
) -> Result<Vec<AcpRegistryAgent>, EngineError> {
    let agents = load_index(data_dir, INDEX_URL).await?;
    Ok(rank(&agents, query.unwrap_or_default(), |agent| {
        installed(data_dir, agent)
    }))
}

/// T3's `searchRank`, matching on id, name and description.
fn search_rank(agent: &Agent, query: &str) -> Option<u8> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(100);
    }
    let id = agent.id.to_lowercase();
    let name = agent.name.to_lowercase();
    let description = agent.description.to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    if id == query || name == query {
        return Some(0);
    }
    if id.starts_with(&query) || name.starts_with(&query) {
        return Some(10);
    }
    let identity = format!("{id} {name}");
    let tokens: Vec<&str> = identity
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    if terms
        .iter()
        .all(|term| tokens.iter().any(|token| token.starts_with(term)))
    {
        return Some(20);
    }
    if terms
        .iter()
        .all(|term| id.contains(term) || name.contains(term))
    {
        return Some(30);
    }
    let haystack = format!("{id} {name} {description}");
    terms
        .iter()
        .all(|term| haystack.contains(term))
        .then_some(50)
}

fn rank(
    agents: &[Agent],
    query: &str,
    installed: impl Fn(&Agent) -> bool,
) -> Vec<AcpRegistryAgent> {
    let mut ranked: Vec<(u8, &Agent, Vec<AcpDistribution>)> = agents
        .iter()
        .filter_map(|agent| {
            let distributions = agent.distributions();
            if distributions.is_empty() {
                return None;
            }
            Some((search_rank(agent, query)?, agent, distributions))
        })
        .collect();
    ranked.sort_by(|(left_rank, left, _), (right_rank, right, _)| {
        left_rank
            .cmp(right_rank)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
    ranked
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .map(|(_, agent, distributions)| AcpRegistryAgent {
            id: agent.id.clone(),
            name: agent.name.clone(),
            version: agent.version.clone(),
            description: agent.description.clone(),
            website: agent.website.clone(),
            repository: agent.repository.clone(),
            distributions,
            icon_url: agent.icon.as_deref().and_then(official_icon_url),
            installed: installed(agent),
        })
        .collect()
}

fn official_icon_url(icon: &str) -> Option<String> {
    let url = reqwest::Url::parse(icon).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some(ICON_HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareRecord {
    agent_id: String,
    version: String,
    name: String,
    #[serde(flatten)]
    launch: RecordLaunch,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "distribution",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum RecordLaunch {
    Npx {
        package: String,
        entry: PathBuf,
    },
    Uvx {
        package: String,
    },
    Binary {
        program: PathBuf,
        archive: String,
        sha256: Option<String>,
        verified: bool,
    },
}

impl RecordLaunch {
    fn distribution(&self) -> AcpDistribution {
        match self {
            RecordLaunch::Npx { .. } => AcpDistribution::Npx,
            RecordLaunch::Uvx { .. } => AcpDistribution::Uvx,
            RecordLaunch::Binary { .. } => AcpDistribution::Binary,
        }
    }
}

fn distribution_name(distribution: AcpDistribution) -> &'static str {
    match distribution {
        AcpDistribution::Npx => "npx",
        AcpDistribution::Uvx => "uvx",
        AcpDistribution::Binary => "binary",
    }
}

fn record_path(
    data_dir: &Path,
    agent_id: &str,
    version: &str,
    distribution: AcpDistribution,
) -> PathBuf {
    registry_dir(data_dir)
        .join("prepared")
        .join(agent_id)
        .join(version)
        .join(format!("{}.json", distribution_name(distribution)))
}

pub(crate) struct PreparedLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

pub(crate) fn prepared_launch(
    data_dir: &Path,
    agent_id: &str,
    version: &str,
    distribution: AcpDistribution,
) -> Result<Option<PreparedLaunch>, EngineError> {
    if !path_safe(agent_id) || !path_safe(version) {
        return Ok(None);
    }
    let Ok(bytes) = std::fs::read(record_path(data_dir, agent_id, version, distribution)) else {
        return Ok(None);
    };
    let Ok(record) = serde_json::from_slice::<PrepareRecord>(&bytes) else {
        return Ok(None);
    };
    if record.agent_id != agent_id
        || record.version != version
        || record.launch.distribution() != distribution
    {
        return Ok(None);
    }
    let (program, args) = match record.launch {
        RecordLaunch::Npx { entry, .. } if entry.is_file() => {
            let (program, mut args) = zeron_harness::npm_tool_launch(&entry)
                .map_err(|error| EngineError::Other(error.to_string()))?;
            args.extend(record.args);
            (program, args)
        }
        RecordLaunch::Uvx { package } => (
            PathBuf::from("uvx"),
            std::iter::once(package).chain(record.args).collect(),
        ),
        RecordLaunch::Binary { program, .. } if program.is_file() => (program, record.args),
        RecordLaunch::Npx { .. } | RecordLaunch::Binary { .. } => return Ok(None),
    };
    Ok(Some(PreparedLaunch {
        program,
        args,
        env: record.env,
    }))
}

fn installed(data_dir: &Path, agent: &Agent) -> bool {
    agent.distributions().into_iter().any(|distribution| {
        matches!(
            prepared_launch(data_dir, &agent.id, &agent.version, distribution),
            Ok(Some(_))
        )
    })
}

pub(crate) async fn prepare(
    data_dir: &Path,
    agent_id: &str,
    distribution: Option<AcpDistribution>,
) -> Result<AcpPreparedAgent, EngineError> {
    let agents = load_index(data_dir, INDEX_URL).await?;
    let agent = agents
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| EngineError::Other(format!("The ACP Registry has no agent '{agent_id}'")))?;
    let offered = agent.distributions();
    let distribution = match distribution {
        Some(distribution) if offered.contains(&distribution) => distribution,
        Some(distribution) => {
            return Err(EngineError::Other(format!(
                "{} has no {} distribution for this device",
                agent.name,
                distribution_name(distribution)
            )));
        }
        None => *offered.first().ok_or_else(|| {
            EngineError::Other(format!(
                "{} has no distribution for this device",
                agent.name
            ))
        })?,
    };
    let record = install(&agent, distribution).await?;
    persist_json(
        &record_path(data_dir, &agent.id, &agent.version, distribution),
        &record,
    )?;
    let icon_path = match agent.icon.as_deref().and_then(official_icon_url) {
        Some(url) => match fetch_icon(data_dir, &agent.id, &url).await {
            Ok(path) => Some(path.display().to_string()),
            Err(error) => {
                tracing::warn!(agent = agent.id, error, "ACP Registry icon not saved");
                None
            }
        },
        None => None,
    };
    Ok(AcpPreparedAgent {
        agent_id: agent.id,
        version: agent.version,
        distribution,
        icon_path,
        display_name: agent.name,
    })
}

async fn install(
    agent: &Agent,
    distribution: AcpDistribution,
) -> Result<PrepareRecord, EngineError> {
    let failed = |error: zeron_harness::HarnessError| EngineError::Other(error.to_string());
    let record = |launch, args, env| PrepareRecord {
        agent_id: agent.id.clone(),
        version: agent.version.clone(),
        name: agent.name.clone(),
        launch,
        args,
        env,
    };
    match distribution {
        AcpDistribution::Binary => {
            let target = agent.binary_target().ok_or_else(|| {
                EngineError::Other(format!("{} has no binary for this device", agent.name))
            })?;
            if !target.archive.starts_with("https://") {
                return Err(EngineError::Other(format!(
                    "{}'s download {} is not https",
                    agent.name, target.archive
                )));
            }
            let name = format!("acp-registry/{}", agent.id);
            let program = zeron_harness::archive_install::install_registry_binary(
                &zeron_harness::archive_install::RegistryBinary {
                    name: &name,
                    version: &agent.version,
                    url: &target.archive,
                    cmd: &target.cmd,
                    sha256: target.sha256.as_deref(),
                },
                &agent.name,
            )
            .await
            .map_err(failed)?;
            let verified = target.sha256.is_some();
            Ok(record(
                RecordLaunch::Binary {
                    program,
                    archive: target.archive,
                    sha256: target.sha256,
                    verified,
                },
                target.args,
                target.env,
            ))
        }
        AcpDistribution::Npx => {
            let package =
                agent.distribution.npx.clone().ok_or_else(|| {
                    EngineError::Other(format!("{} has no npx package", agent.name))
                })?;
            let entry = zeron_harness::ensure_npm_tool(
                &package.package,
                npm_bin_name(&package.package),
                &agent.name,
            )
            .await
            .map_err(failed)?;
            Ok(record(
                RecordLaunch::Npx {
                    package: package.package,
                    entry,
                },
                package.args,
                package.env,
            ))
        }
        AcpDistribution::Uvx => {
            let package =
                agent.distribution.uvx.clone().ok_or_else(|| {
                    EngineError::Other(format!("{} has no uvx package", agent.name))
                })?;
            if zeron_harness::acp::find_uvx().is_none() {
                return Err(EngineError::Other(format!(
                    "{} runs through uv, which is not installed. Install uv (https://docs.astral.sh/uv/) and try again.",
                    agent.name
                )));
            }
            Ok(record(
                RecordLaunch::Uvx {
                    package: package.package,
                },
                package.args,
                package.env,
            ))
        }
    }
}

fn npm_bin_name(package: &str) -> &str {
    let name = match package.rfind('@') {
        Some(at) if at > 0 => &package[..at],
        _ => package,
    };
    name.rsplit('/').next().unwrap_or(name)
}

async fn fetch_icon(data_dir: &Path, agent_id: &str, url: &str) -> Result<PathBuf, String> {
    let response = http()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ICON_BYTES as u64)
    {
        return Err(format!("the icon is larger than {MAX_ICON_BYTES} bytes"));
    }
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    save_icon(data_dir, agent_id, &bytes)
}

fn save_icon(data_dir: &Path, agent_id: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err(format!("the icon is larger than {MAX_ICON_BYTES} bytes"));
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    usvg::Tree::from_data(bytes, &options).map_err(|error| error.to_string())?;
    let dir = data_dir.join("registry-icons");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{agent_id}.svg"));
    let temp = dir.join(format!(".{agent_id}.svg.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, &path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zeron_proto::{AcpSource, HarnessId, InstanceConfig, ProviderInstance};

    use super::*;

    fn index(platform: &str) -> String {
        json!({
            "version": "1.0.0",
            "extensions": [{"future": true}],
            "agents": [
                {
                    "id": "gamma-code", "name": "Gamma Code", "version": "1.0.0",
                    "description": "Writes code", "license": "MIT", "authors": ["x"],
                    "icon": "https://cdn.agentclientprotocol.com/registry/v1/latest/gamma-code.svg",
                    "distribution": {"npx": {"package": "@gamma/code@1.0.0", "args": ["--acp"], "future": 1}}
                },
                {"id": "broken", "version": "1.0.0", "description": "no name", "distribution": {"npx": {"package": "b@1"}}},
                {"id": "wrong-shape", "name": "Wrong", "version": "1.0.0", "distribution": {"npx": "not-an-object"}},
                {"id": "../escape", "name": "Escape", "version": "1.0.0", "distribution": {"npx": {"package": "e@1"}}},
                {
                    "id": "windows-only", "name": "Windows Only", "version": "1.0.0", "description": "x",
                    "distribution": {"binary": {"windows-x86_64": {"archive": "https://x/w.zip", "cmd": "w.exe"}}}
                },
                {
                    "id": "alpha", "name": "Alpha", "version": "2.0.0", "description": "A gamma helper",
                    "icon": "http://cdn.agentclientprotocol.com/alpha.svg",
                    "distribution": {
                        "binary": {
                            platform: {"archive": "https://x/a.tar.gz", "cmd": "./bin/alpha", "sha256": "ab"},
                            "linux-riscv64": {"archive": 7}
                        },
                        "uvx": {"package": "alpha==2.0.0"}
                    }
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn the_index_parses_leniently_and_offers_only_this_platforms_agents() {
        let Some(platform) = platform_key() else {
            return;
        };
        let agents = parse_index(&index(platform)).unwrap();
        let ids: Vec<&str> = agents.iter().map(|agent| agent.id.as_str()).collect();
        assert_eq!(ids, ["gamma-code", "windows-only", "alpha"]);
        assert!(parse_index("{\"version\":\"1.0.0\"}").is_err());

        let all = rank(&agents, "", |agent| agent.id == "alpha");
        let rows: Vec<(&str, &[AcpDistribution], bool)> = all
            .iter()
            .map(|row| (row.id.as_str(), row.distributions.as_slice(), row.installed))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "alpha",
                    &[AcpDistribution::Binary, AcpDistribution::Uvx][..],
                    true
                ),
                ("gamma-code", &[AcpDistribution::Npx][..], false),
            ]
        );
        assert_eq!(
            all[0].icon_url, None,
            "only https icons on the registry CDN"
        );
        assert_eq!(
            all[1].icon_url.as_deref(),
            Some("https://cdn.agentclientprotocol.com/registry/v1/latest/gamma-code.svg")
        );

        let gamma: Vec<String> = rank(&agents, "GAMMA", |_| false)
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            gamma,
            ["gamma-code", "alpha"],
            "a name prefix outranks a description hit"
        );
        assert!(rank(&agents, "nothing matches", |_| false).is_empty());
    }

    fn registry_instance(version: &str, distribution: AcpDistribution) -> ProviderInstance {
        ProviderInstance {
            id: "acp_demo".into(),
            driver: HarnessId::Acp,
            label: "Demo".into(),
            accent: None,
            config: InstanceConfig::Acp {
                source: AcpSource::Registry {
                    agent_id: "demo".into(),
                    version: version.into(),
                    distribution,
                },
                env: [("SHARED".to_owned(), "instance".to_owned())].into(),
                icon_path: None,
            },
        }
    }

    fn write_record(data_dir: &Path, distribution: AcpDistribution, launch: Value) {
        let mut record = json!({
            "agentId": "demo",
            "version": "1.0.0",
            "name": "Demo",
            "args": ["--acp"],
            "env": {"SHARED": "registry", "ONLY_REGISTRY": "1"}
        });
        record
            .as_object_mut()
            .unwrap()
            .extend(launch.as_object().unwrap().clone());
        let path = record_path(data_dir, "demo", "1.0.0", distribution);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, record.to_string()).unwrap();
    }

    #[test]
    fn a_registry_instance_launches_from_its_prepare_record() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path();
        let launch = |version, distribution| {
            crate::provider_instances::acp_launch(
                &registry_instance(version, distribution),
                data_dir,
            )
        };
        for distribution in [
            AcpDistribution::Npx,
            AcpDistribution::Uvx,
            AcpDistribution::Binary,
        ] {
            let error = launch("1.0.0", distribution).unwrap_err().to_string();
            assert!(error.contains("'demo' is not prepared"), "{error}");
        }

        let program = temp.path().join("install/bin/demo");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        let entry = temp.path().join("npm/node_modules/demo/cli");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]).unwrap();
        write_record(
            data_dir,
            AcpDistribution::Binary,
            json!({"distribution": "binary", "program": program, "archive": "https://x/demo.tar.gz", "sha256": null, "verified": false}),
        );
        write_record(
            data_dir,
            AcpDistribution::Npx,
            json!({"distribution": "npx", "package": "demo@1.0.0", "entry": entry}),
        );
        write_record(
            data_dir,
            AcpDistribution::Uvx,
            json!({"distribution": "uvx", "package": "demo==1.0.0"}),
        );

        let expected_env: BTreeMap<String, String> = [
            ("ONLY_REGISTRY".to_owned(), "1".to_owned()),
            ("SHARED".to_owned(), "instance".to_owned()),
        ]
        .into();
        for (distribution, program, args) in [
            (AcpDistribution::Binary, program.clone(), vec!["--acp"]),
            (AcpDistribution::Npx, entry.clone(), vec!["--acp"]),
            (
                AcpDistribution::Uvx,
                PathBuf::from("uvx"),
                vec!["demo==1.0.0", "--acp"],
            ),
        ] {
            let resolved = launch("1.0.0", distribution).unwrap();
            assert_eq!(resolved.display_name, "Demo");
            assert_eq!(resolved.program, program, "{distribution:?}");
            assert_eq!(resolved.args, args, "{distribution:?}");
            assert_eq!(resolved.env, expected_env, "{distribution:?}");
            let error = launch("2.0.0", distribution).unwrap_err().to_string();
            assert!(error.contains("not prepared"), "{distribution:?}: {error}");
        }

        std::fs::remove_file(&program).unwrap();
        let error = launch("1.0.0", AcpDistribution::Binary)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not prepared"), "a removed install: {error}");
        std::fs::copy(
            record_path(data_dir, "demo", "1.0.0", AcpDistribution::Uvx),
            record_path(data_dir, "demo", "1.0.0", AcpDistribution::Npx),
        )
        .unwrap();
        let error = launch("1.0.0", AcpDistribution::Npx)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not prepared"),
            "a record of another distribution: {error}"
        );
    }

    #[tokio::test]
    #[ignore = "live: fetches the real ACP Registry and installs two agents"]
    async fn live_registry_smoke() {
        let data_dir = PathBuf::from(std::env::var("ZERON_SMOKE_DATA_DIR").unwrap());
        assert!(std::env::var_os("ZERON_ADAPTERS_DIR").is_some());
        let all = search(&data_dir, None).await.unwrap();
        let first: Vec<_> = all
            .iter()
            .take(6)
            .map(|row| (&row.id, &row.distributions))
            .collect();
        println!("search all: {} rows, first {first:?}", all.len());
        let index_path = registry_dir(&data_dir).join("index.json");
        let cached: CachedIndex =
            serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
        let revalidated = fetch_index(INDEX_URL, cached.etag.as_deref())
            .await
            .unwrap();
        println!(
            "cached etag {:?}; If-None-Match answer: {}",
            cached.etag,
            match revalidated {
                Fetched::NotModified => "304 Not Modified".to_owned(),
                Fetched::Body { .. } => "200 with a new body".to_owned(),
            }
        );
        let stale = CachedIndex {
            fetched_at: 0,
            ..cached
        };
        persist_json(&index_path, &stale).unwrap();
        let offline = load_index(&data_dir, "http://127.0.0.1:9/registry.json")
            .await
            .unwrap();
        println!(
            "offline with a stale cache: {} agents served",
            offline.len()
        );

        for (agent_id, distribution) in [("pi-acp", None), ("kimchi", None)] {
            let prepared = prepare(&data_dir, agent_id, distribution).await.unwrap();
            println!("prepared: {prepared:?}");
            let record = record_path(
                &data_dir,
                &prepared.agent_id,
                &prepared.version,
                prepared.distribution,
            );
            println!(
                "record {}:\n{}",
                record.display(),
                std::fs::read_to_string(&record).unwrap()
            );
            let instance = ProviderInstance {
                id: format!("acp_{}", agent_id.replace('-', "_")),
                driver: HarnessId::Acp,
                label: prepared.display_name.clone(),
                accent: None,
                config: InstanceConfig::Acp {
                    source: AcpSource::Registry {
                        agent_id: prepared.agent_id.clone(),
                        version: prepared.version.clone(),
                        distribution: prepared.distribution,
                    },
                    env: BTreeMap::new(),
                    icon_path: prepared.icon_path.clone(),
                },
            };
            let launch = crate::provider_instances::acp_launch(&instance, &data_dir).unwrap();
            println!("launch: {launch:?}");
            let methods = zeron_harness::AcpHarness::generic(launch)
                .unwrap()
                .auth_methods()
                .await;
            println!("initialize -> authMethods: {methods:?}");
        }
        let kimchi = search(&data_dir, Some("kimchi")).await.unwrap();
        println!("search kimchi: {kimchi:?}");
    }
}
