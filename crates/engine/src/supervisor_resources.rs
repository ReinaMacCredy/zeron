use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use zeron_proto::supervisor::{
    CheckTool, ResourceCheck, ResourceKey, ResourceState, ResourceUse, SharedResource,
};

pub(crate) const SLOT_PERCENT: f32 = 50.0;
pub(crate) const RECHECK_MS: i64 = 2 * 60 * 1000;
pub(crate) const PAGE_CHECK_MS: i64 = 60 * 1000;
pub(crate) const AFTER_PAUSE_MS: [i64; 2] = [30_000, 2 * 60 * 1000];
const SLOT_SAMPLE_GAP: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const USES_PER_OWNER: usize = 8;
pub(crate) const MAX_KEYS: usize = 16;

pub(crate) fn checked_keys(keys: &[String]) -> Result<Vec<String>, String> {
    let mut parsed: Vec<ResourceKey> = Vec::new();
    for key in keys {
        let key = ResourceKey::parse(key).ok_or_else(|| {
            format!("\"{key}\" is not a resource: use port:<1-65535>, slot:<name> or path:<path inside the project>")
        })?;
        if !parsed.contains(&key) {
            parsed.push(key);
        }
    }
    if parsed.len() > MAX_KEYS {
        return Err(format!("A mission declares at most {MAX_KEYS} resources"));
    }
    Ok(parsed.iter().map(ResourceKey::to_string).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LsofProcess {
    pub pid: u32,
    pub command: Option<String>,
    pub names: Vec<String>,
}

pub(crate) fn parse_lsof(out: &str) -> Vec<LsofProcess> {
    let mut processes: Vec<LsofProcess> = Vec::new();
    let mut current = false;
    for line in out.lines() {
        let Some(field) = line.chars().next() else {
            continue;
        };
        let value = &line[field.len_utf8()..];
        match field {
            'p' => {
                current = match value.parse() {
                    Ok(pid) => {
                        processes.push(LsofProcess {
                            pid,
                            command: None,
                            names: Vec::new(),
                        });
                        true
                    }
                    Err(_) => false,
                };
            }
            'c' if current => {
                if let Some(process) = processes.last_mut() {
                    process.command = Some(value.to_string());
                }
            }
            'n' if current => {
                if let Some(process) = processes.last_mut() {
                    process.names.push(value.to_string());
                }
            }
            _ => {}
        }
    }
    processes
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub cpu: f32,
    pub command: String,
}

pub(crate) fn parse_ps(out: &str) -> Vec<PsRow> {
    out.lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut field = || {
                let end = rest.find(char::is_whitespace)?;
                let (value, tail) = rest.split_at(end);
                rest = tail.trim_start();
                Some(value)
            };
            let pid = field()?.parse().ok()?;
            let ppid = field()?.parse().ok()?;
            let cpu = field()?.replace(',', ".").parse().ok()?;
            let command = rest.trim();
            (!command.is_empty()).then(|| PsRow {
                pid,
                ppid,
                cpu,
                command: command.to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ResourceMission {
    pub task_id: String,
    pub keys: Vec<ResourceKey>,
    pub active: bool,
    pub threads: Vec<String>,
    pub cwds: Vec<String>,
    pub checkouts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub key: ResourceKey,
    pub holders: Vec<String>,
    pub others: Vec<String>,
}

impl Candidate {
    pub fn verifiable(&self) -> bool {
        !matches!(self.key, ResourceKey::Path(_))
    }

    fn involves(&self, task_id: &str) -> bool {
        self.holders
            .iter()
            .chain(&self.others)
            .any(|id| id == task_id)
    }
}

pub(crate) fn find_candidates(missions: &[ResourceMission]) -> Vec<Candidate> {
    let holding: Vec<&ResourceMission> = missions
        .iter()
        .filter(|mission| mission.active && !mission.keys.is_empty())
        .collect();
    let declaring = |key: &ResourceKey| -> Vec<String> {
        holding
            .iter()
            .filter(|mission| mission.keys.contains(key))
            .map(|mission| mission.task_id.clone())
            .collect()
    };
    let mut found: BTreeMap<ResourceKey, Candidate> = BTreeMap::new();
    let keys: BTreeSet<&ResourceKey> = holding.iter().flat_map(|mission| &mission.keys).collect();
    for key in keys {
        let holders = declaring(key);
        let others: Vec<String> = match key {
            ResourceKey::Port(_) if holders.len() >= 2 => Vec::new(),
            ResourceKey::Slot(_) => missions
                .iter()
                .filter(|mission| mission.active && !holders.contains(&mission.task_id))
                .map(|mission| mission.task_id.clone())
                .collect(),
            _ => continue,
        };
        if holders.len() + others.len() >= 2 {
            found.insert(
                key.clone(),
                Candidate {
                    key: key.clone(),
                    holders,
                    others,
                },
            );
        }
    }
    let paths = |mission: &&ResourceMission| -> Vec<ResourceKey> {
        mission
            .keys
            .iter()
            .filter(|key| matches!(key, ResourceKey::Path(_)))
            .cloned()
            .collect()
    };
    for (at, a) in holding.iter().enumerate() {
        for b in &holding[at + 1..] {
            if !a
                .checkouts
                .iter()
                .any(|checkout| b.checkouts.contains(checkout))
            {
                continue;
            }
            for outer in paths(a) {
                for inner in paths(b).into_iter().filter(|key| outer.overlaps(key)) {
                    let key = if outer.to_string().len() <= inner.to_string().len() {
                        outer.clone()
                    } else {
                        inner
                    };
                    let entry = found.entry(key.clone()).or_insert_with(|| Candidate {
                        key,
                        holders: Vec::new(),
                        others: Vec::new(),
                    });
                    for id in [&a.task_id, &b.task_id] {
                        if !entry.holders.contains(id) {
                            entry.holders.push(id.clone());
                        }
                    }
                }
            }
        }
    }
    found
        .into_values()
        .map(|mut candidate| {
            candidate.holders.sort();
            candidate.others.sort();
            candidate
        })
        .collect()
}

struct Roots(Vec<(PathBuf, String)>);

impl Roots {
    fn of<'a>(missions: impl Iterator<Item = &'a ResourceMission>) -> Self {
        let home = crate::repos::expand_home("~").ok().map(PathBuf::from);
        let mut roots = Vec::new();
        for mission in missions {
            for cwd in &mission.cwds {
                let path = PathBuf::from(crate::repos::expand_home(cwd).unwrap_or(cwd.clone()));
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                if path.parent().is_none() || Some(&path) == home.as_ref() {
                    continue;
                }
                roots.push((path, mission.task_id.clone()));
            }
        }
        Self(roots)
    }

    fn owners(&self, cwd: &Path) -> Vec<String> {
        let owners: BTreeSet<&String> = self
            .0
            .iter()
            .filter(|(root, _)| cwd.starts_with(root))
            .map(|(_, task)| task)
            .collect();
        owners.into_iter().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sampled {
    pub pid: u32,
    pub cpu: f32,
    pub command: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Load {
    pub owners: Vec<String>,
    pub cwd: Option<String>,
    pub cpu: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Verdict {
    pub check: ResourceCheck,
    pub missions: Vec<String>,
    pub blocked: Option<String>,
    pub loads: Vec<Load>,
}

fn failed(at: DateTime<Utc>, tool: CheckTool, error: &str) -> ResourceCheck {
    ResourceCheck {
        at,
        tool,
        contended: false,
        error: Some(error.to_string()),
        uses: Vec::new(),
    }
}

fn short(command: &str) -> String {
    Path::new(command)
        .file_name()
        .map_or(command, |name| name.to_str().unwrap_or(command))
        .to_string()
}

fn single(owners: &[String]) -> Option<String> {
    match owners {
        [one] => Some(one.clone()),
        _ => None,
    }
}

fn port_verdict(
    candidate: &Candidate,
    listeners: Option<&[LsofProcess]>,
    cwds: &HashMap<u32, PathBuf>,
    roots: &Roots,
    at: DateTime<Utc>,
) -> Verdict {
    let Some(listeners) = listeners else {
        return Verdict {
            check: failed(at, CheckTool::Lsof, "lsof did not run"),
            missions: candidate.holders.clone(),
            blocked: None,
            loads: Vec::new(),
        };
    };
    let mut missions = candidate.holders.clone();
    let mut owning = HashSet::new();
    let uses: Vec<ResourceUse> = listeners
        .iter()
        .map(|process| {
            let cwd = cwds.get(&process.pid);
            let owners = cwd.map(|cwd| roots.owners(cwd)).unwrap_or_default();
            for owner in &owners {
                owning.insert(owner.clone());
                if !missions.contains(owner) {
                    missions.push(owner.clone());
                }
            }
            ResourceUse {
                pid: process.pid,
                command: process.command.clone().unwrap_or_default(),
                cwd: cwd.map(|cwd| cwd.display().to_string()),
                task_id: single(&owners),
                cpu: None,
            }
        })
        .collect();
    let blocked = candidate
        .holders
        .iter()
        .find(|holder| !owning.contains(*holder))
        .or(candidate.holders.first())
        .cloned();
    Verdict {
        check: ResourceCheck {
            at,
            tool: CheckTool::Lsof,
            contended: !uses.is_empty(),
            error: None,
            uses,
        },
        missions,
        blocked,
        loads: Vec::new(),
    }
}

pub(crate) fn slot_verdict(
    candidate: &Candidate,
    owners: &dyn Fn(&Path) -> Vec<String>,
    first: &[Sampled],
    second: &[Sampled],
    at: DateTime<Utc>,
) -> Verdict {
    let bucket = |sampled: &Sampled| -> Vec<String> {
        sampled.cwd.as_deref().map(owners).unwrap_or_default()
    };
    let load = |sample: &[Sampled]| {
        let mut sums: BTreeMap<Vec<String>, f32> = BTreeMap::new();
        for sampled in sample {
            let owners = bucket(sampled);
            if !owners.is_empty() {
                *sums.entry(owners).or_default() += sampled.cpu;
            }
        }
        sums
    };
    let (before, after) = (load(first), load(second));
    let mut loads: Vec<Load> = Vec::new();
    let mut blocked = None;
    for holder in &candidate.holders {
        for (owners, cpu) in &after {
            let earlier = before.get(owners).copied().unwrap_or_default();
            if owners.as_slice() == std::slice::from_ref(holder)
                || earlier <= SLOT_PERCENT
                || *cpu <= SLOT_PERCENT
            {
                continue;
            }
            blocked.get_or_insert_with(|| holder.clone());
            if !loads.iter().any(|load| &load.owners == owners) {
                loads.push(Load {
                    owners: owners.clone(),
                    cwd: None,
                    cpu: earlier.min(*cpu),
                });
            }
        }
    }
    let mut uses = Vec::new();
    for load in &mut loads {
        let mut busiest: Vec<&Sampled> = second
            .iter()
            .filter(|sampled| bucket(sampled) == load.owners)
            .collect();
        busiest.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        load.cwd = busiest
            .first()
            .and_then(|sampled| sampled.cwd.as_ref())
            .map(|cwd| cwd.display().to_string());
        uses.extend(
            busiest
                .into_iter()
                .take(USES_PER_OWNER)
                .map(|sampled| ResourceUse {
                    pid: sampled.pid,
                    command: sampled.command.clone(),
                    cwd: sampled.cwd.as_ref().map(|cwd| cwd.display().to_string()),
                    task_id: single(&load.owners),
                    cpu: Some(sampled.cpu),
                }),
        );
    }
    let mut missions = candidate.holders.clone();
    for owner in loads.iter().flat_map(|load| &load.owners) {
        if !missions.contains(owner) {
            missions.push(owner.clone());
        }
    }
    Verdict {
        check: ResourceCheck {
            at,
            tool: CheckTool::Ps,
            contended: !loads.is_empty(),
            error: None,
            uses,
        },
        missions,
        blocked,
        loads,
    }
}

async fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(COMMAND_TIMEOUT, output)
        .await
        .ok()?
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn listeners(port: u16) -> Option<Vec<LsofProcess>> {
    let filter = format!("-iTCP:{port}");
    let out = run("lsof", &["-nP", &filter, "-sTCP:LISTEN", "-Fpc"]).await?;
    Some(parse_lsof(&out))
}

async fn cwds(pids: &[u32]) -> HashMap<u32, PathBuf> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    let Some(out) = run("lsof", &["-a", "-p", &list.join(","), "-d", "cwd", "-Fn"]).await else {
        return HashMap::new();
    };
    parse_lsof(&out)
        .into_iter()
        .filter_map(|process| Some((process.pid, PathBuf::from(process.names.first()?))))
        .collect()
}

async fn process_table() -> Option<Vec<PsRow>> {
    let out = run("ps", &["-A", "-o", "pid=,ppid=,%cpu=,comm="]).await?;
    Some(parse_ps(&out))
}

async fn cpu_sample() -> Option<Vec<Sampled>> {
    let rows = process_table().await?;
    let busy: Vec<&PsRow> = rows.iter().filter(|row| row.cpu > 0.0).collect();
    let mut asked: Vec<u32> = busy
        .iter()
        .flat_map(|row| [row.pid, row.ppid])
        .filter(|pid| *pid > 1)
        .collect();
    asked.sort_unstable();
    asked.dedup();
    let cwds = cwds(&asked).await;
    Some(
        busy.into_iter()
            .map(|row| Sampled {
                pid: row.pid,
                cpu: row.cpu,
                command: short(&row.command),
                cwd: cwds.get(&row.pid).or_else(|| cwds.get(&row.ppid)).cloned(),
            })
            .collect(),
    )
}

#[derive(Debug, Clone)]
pub(crate) struct Probe {
    pub row_id: String,
    pub key: ResourceKey,
    pub pids: Vec<u32>,
}

pub(crate) struct CheckInput {
    pub missions: Vec<ResourceMission>,
    pub candidates: Vec<Candidate>,
    pub probes: Vec<Probe>,
}

pub(crate) struct CheckOutcome {
    pub verdicts: Vec<(ResourceKey, Verdict)>,
    pub freed: Vec<String>,
}

pub(crate) async fn run_check(input: CheckInput) -> CheckOutcome {
    let ports: BTreeSet<u16> = input
        .candidates
        .iter()
        .map(|candidate| &candidate.key)
        .chain(input.probes.iter().map(|probe| &probe.key))
        .filter_map(|key| match key {
            ResourceKey::Port(port) => Some(*port),
            _ => None,
        })
        .collect();
    let mut listening: HashMap<u16, Option<Vec<LsofProcess>>> = HashMap::new();
    for port in ports {
        listening.insert(port, listeners(port).await);
    }
    let mut listener_pids: Vec<u32> = listening
        .values()
        .flatten()
        .flatten()
        .map(|process| process.pid)
        .collect();
    listener_pids.sort_unstable();
    listener_pids.dedup();
    let listener_cwds = cwds(&listener_pids).await;
    let slots = input
        .candidates
        .iter()
        .any(|candidate| matches!(candidate.key, ResourceKey::Slot(_)));
    let samples = if slots {
        let first = cpu_sample().await;
        tokio::time::sleep(SLOT_SAMPLE_GAP).await;
        first.zip(cpu_sample().await)
    } else {
        None
    };
    let at = Utc::now();
    let verdicts = input
        .candidates
        .iter()
        .map(|candidate| {
            let roots = Roots::of(
                input
                    .missions
                    .iter()
                    .filter(|mission| candidate.involves(&mission.task_id)),
            );
            let verdict = match &candidate.key {
                ResourceKey::Port(port) => port_verdict(
                    candidate,
                    listening.get(port).and_then(Option::as_deref),
                    &listener_cwds,
                    &roots,
                    at,
                ),
                _ => match &samples {
                    Some((first, second)) => slot_verdict(
                        candidate,
                        &|cwd: &Path| roots.owners(cwd),
                        first,
                        second,
                        at,
                    ),
                    None => Verdict {
                        check: failed(at, CheckTool::Ps, "ps or lsof did not run"),
                        missions: candidate.holders.clone(),
                        blocked: None,
                        loads: Vec::new(),
                    },
                },
            };
            (candidate.key.clone(), verdict)
        })
        .collect();
    let alive = if input
        .probes
        .iter()
        .any(|probe| !matches!(probe.key, ResourceKey::Port(_)))
    {
        process_table()
            .await
            .map(|rows| rows.iter().map(|row| row.pid).collect::<HashSet<u32>>())
    } else {
        None
    };
    let freed =
        input
            .probes
            .iter()
            .filter(|probe| match &probe.key {
                ResourceKey::Port(port) => listening
                    .get(port)
                    .and_then(Option::as_ref)
                    .is_some_and(|open| {
                        if probe.pids.is_empty() {
                            open.is_empty()
                        } else {
                            !open.iter().any(|process| probe.pids.contains(&process.pid))
                        }
                    }),
                _ => alive.as_ref().is_some_and(|alive| {
                    !probe.pids.is_empty() && !probe.pids.iter().any(|pid| alive.contains(pid))
                }),
            })
            .map(|probe| probe.row_id.clone())
            .collect();
    CheckOutcome { verdicts, freed }
}

#[derive(Debug, Clone)]
pub(crate) struct Contention {
    pub since: i64,
    pub missions: Vec<String>,
    pub blocked: Option<String>,
    pub check: ResourceCheck,
    pub loads: Vec<Load>,
}

#[derive(Default)]
pub(crate) struct Resources {
    missions: Vec<ResourceMission>,
    candidates: Vec<Candidate>,
    checks: HashMap<ResourceKey, ResourceCheck>,
    open: HashMap<ResourceKey, Contention>,
    last_check: Option<i64>,
    due: Vec<i64>,
    wanted: bool,
    rows: Vec<SharedResource>,
}

impl Resources {
    pub fn observe(
        &mut self,
        missions: Vec<ResourceMission>,
        candidates: Vec<Candidate>,
        edges: &HashSet<String>,
        now: i64,
    ) -> Option<i64> {
        if candidates
            .iter()
            .any(|candidate| candidate.verifiable() && !self.candidates.contains(candidate))
        {
            self.wanted = true;
        }
        let current: HashSet<&ResourceKey> = candidates.iter().map(|c| &c.key).collect();
        self.checks.retain(|key, _| current.contains(key));
        self.open.retain(|key, _| current.contains(key));
        let touched = self.open.values().any(|open| {
            missions
                .iter()
                .filter(|mission| open.missions.contains(&mission.task_id))
                .any(|mission| mission.threads.iter().any(|thread| edges.contains(thread)))
        });
        if touched && self.last_check.is_none_or(|at| now - at >= RECHECK_MS) {
            self.wanted = true;
        }
        let before = self.due.len();
        self.due.retain(|at| *at > now);
        if self.due.len() < before {
            self.wanted = true;
        }
        self.missions = missions;
        self.candidates = candidates;
        self.rows = self.derive_rows();
        self.due.iter().min().copied()
    }

    pub fn wanted(&self) -> bool {
        self.wanted
    }

    pub fn request_if_older(&mut self, now: i64, age: i64) {
        if self.last_check.is_none_or(|at| now - at >= age) {
            self.wanted = true;
        }
    }

    pub fn schedule(&mut self, at: i64) {
        self.due.push(at);
    }

    pub fn take_input(&mut self, probes: Vec<Probe>) -> Option<CheckInput> {
        self.wanted = false;
        let candidates: Vec<Candidate> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.verifiable())
            .cloned()
            .collect();
        if candidates.is_empty() && probes.is_empty() {
            return None;
        }
        Some(CheckInput {
            missions: self.missions.clone(),
            candidates,
            probes,
        })
    }

    pub fn apply(
        &mut self,
        verdicts: Vec<(ResourceKey, Verdict)>,
        now: i64,
    ) -> Vec<(ResourceKey, Contention)> {
        self.last_check = Some(now);
        let mut opened = Vec::new();
        for (key, verdict) in verdicts {
            if !self.candidates.iter().any(|candidate| candidate.key == key) {
                continue;
            }
            if verdict.check.contended {
                let earlier = self.open.get(&key).map(|open| open.since);
                let contention = Contention {
                    since: earlier.unwrap_or(now),
                    missions: verdict.missions,
                    blocked: verdict.blocked,
                    check: verdict.check.clone(),
                    loads: verdict.loads,
                };
                if earlier.is_none() {
                    opened.push((key.clone(), contention.clone()));
                }
                self.open.insert(key.clone(), contention);
            } else if verdict.check.error.is_none() {
                self.open.remove(&key);
            }
            self.checks.insert(key, verdict.check);
        }
        self.rows = self.derive_rows();
        opened
    }

    pub fn open(&self, key: &ResourceKey) -> Option<&Contention> {
        self.open.get(key)
    }

    pub fn rows(&self) -> &[SharedResource] {
        &self.rows
    }

    pub fn checked(&self) -> bool {
        self.candidates.iter().any(Candidate::verifiable)
    }

    fn derive_rows(&self) -> Vec<SharedResource> {
        let mut keys: BTreeMap<&ResourceKey, (Vec<String>, Vec<String>)> = BTreeMap::new();
        for mission in &self.missions {
            for key in &mission.keys {
                let (declared, holders) = keys.entry(key).or_default();
                declared.push(mission.task_id.clone());
                if mission.active {
                    holders.push(mission.task_id.clone());
                }
            }
        }
        keys.into_iter()
            .map(|(key, (mut declared_by, mut holders))| {
                declared_by.sort();
                holders.sort();
                let candidate = self.candidates.iter().find(|c| &c.key == key);
                let path_overlap = matches!(key, ResourceKey::Path(_))
                    && self.candidates.iter().any(|c| {
                        matches!(c.key, ResourceKey::Path(_))
                            && c.key.overlaps(key)
                            && c.holders.iter().any(|holder| holders.contains(holder))
                    });
                let check = candidate
                    .filter(|candidate| candidate.verifiable())
                    .and_then(|_| self.checks.get(key))
                    .cloned();
                let state = if path_overlap {
                    ResourceState::NotVerified
                } else if candidate.is_some() {
                    match &check {
                        _ if self.open.contains_key(key) => ResourceState::Contended,
                        None => ResourceState::Checking,
                        Some(check) if check.error.is_some() => ResourceState::CheckFailed,
                        Some(_) => ResourceState::Clear,
                    }
                } else if holders.is_empty() {
                    ResourceState::Free
                } else {
                    ResourceState::Held
                };
                SharedResource {
                    key: key.to_string(),
                    declared_by,
                    holders,
                    state,
                    check,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsof_and_ps_output_parse_into_processes() {
        let lsof_cases: [(&str, &str, Vec<LsofProcess>); 4] = [
            (
                "listener",
                "p35663\ncPython\nf3\n",
                vec![LsofProcess {
                    pid: 35663,
                    command: Some("Python".into()),
                    names: vec![],
                }],
            ),
            (
                "cwds of two processes",
                "p35659\nfcwd\nn/Users/r/Code/zeron fork\np35663\nfcwd\nn/private/tmp/w341-a\n",
                vec![
                    LsofProcess {
                        pid: 35659,
                        command: None,
                        names: vec!["/Users/r/Code/zeron fork".into()],
                    },
                    LsofProcess {
                        pid: 35663,
                        command: None,
                        names: vec!["/private/tmp/w341-a".into()],
                    },
                ],
            ),
            (
                "a bad pid drops its fields",
                "pnope\ncghost\nn/ghost\np7\ncnode\nf12\nn*:5173\n",
                vec![LsofProcess {
                    pid: 7,
                    command: Some("node".into()),
                    names: vec!["*:5173".into()],
                }],
            ),
            ("nothing listens", "", vec![]),
        ];
        for (name, out, expected) in lsof_cases {
            assert_eq!(parse_lsof(out), expected, "{name}");
        }
        let ps = "    1     0  16.5 /sbin/launchd\n\
                  4242   812 101,3 /Applications/Bench Tool.app/Contents/MacOS/bench\n\
                  junk line\n\
                  77 1 0.0\n\
                  9001  4242   0.0 python3\n";
        assert_eq!(
            parse_ps(ps),
            [
                PsRow {
                    pid: 1,
                    ppid: 0,
                    cpu: 16.5,
                    command: "/sbin/launchd".into(),
                },
                PsRow {
                    pid: 4242,
                    ppid: 812,
                    cpu: 101.3,
                    command: "/Applications/Bench Tool.app/Contents/MacOS/bench".into(),
                },
                PsRow {
                    pid: 9001,
                    ppid: 4242,
                    cpu: 0.0,
                    command: "python3".into(),
                },
            ]
        );
    }

    fn mission(task: &str, keys: &[&str], active: bool, checkout: &str) -> ResourceMission {
        ResourceMission {
            task_id: task.into(),
            keys: keys
                .iter()
                .map(|key| ResourceKey::parse(key).unwrap())
                .collect(),
            active,
            threads: vec![format!("{task}-lead")],
            cwds: vec![format!("/tmp/{checkout}")],
            checkouts: vec![checkout.into()],
        }
    }

    fn candidate(key: &str, holders: &[&str], others: &[&str]) -> Candidate {
        Candidate {
            key: ResourceKey::parse(key).unwrap(),
            holders: holders.iter().map(|id| id.to_string()).collect(),
            others: others.iter().map(|id| id.to_string()).collect(),
        }
    }

    #[test]
    fn candidates_come_from_declarations_and_activity_alone() {
        let cases: [(&str, Vec<ResourceMission>, Vec<Candidate>); 8] = [
            (
                "nothing held",
                vec![
                    mission("a", &["port:5173"], false, "x"),
                    mission("b", &["port:5173"], false, "y"),
                ],
                vec![],
            ),
            (
                "one port holder",
                vec![
                    mission("a", &["port:5173"], true, "x"),
                    mission("b", &[], true, "y"),
                ],
                vec![],
            ),
            (
                "a port shared by two holders",
                vec![
                    mission("b", &["port:5173"], true, "y"),
                    mission("a", &["port:5173", "port:8080"], true, "x"),
                ],
                vec![candidate("port:5173", &["a", "b"], &[])],
            ),
            (
                "a port whose other declarer is idle",
                vec![
                    mission("a", &["port:5173"], true, "x"),
                    mission("b", &["port:5173"], false, "y"),
                ],
                vec![],
            ),
            (
                "a slot alone",
                vec![
                    mission("a", &["slot:benchmark"], true, "x"),
                    mission("b", &[], false, "y"),
                ],
                vec![],
            ),
            (
                "a slot while another mission works",
                vec![
                    mission("a", &["slot:benchmark"], true, "x"),
                    mission("b", &[], true, "y"),
                    mission("c", &["slot:benchmark"], false, "z"),
                ],
                vec![candidate("slot:benchmark", &["a"], &["b"])],
            ),
            (
                "two slot holders",
                vec![
                    mission("a", &["slot:benchmark"], true, "x"),
                    mission("b", &["slot:benchmark"], true, "y"),
                ],
                vec![candidate("slot:benchmark", &["a", "b"], &[])],
            ),
            (
                "paths overlap only in one checkout",
                vec![
                    mission("a", &["path:src"], true, "x"),
                    mission("b", &["path:src/checkout"], true, "x"),
                    mission("c", &["path:src"], true, "y"),
                    mission("d", &["path:srcs"], true, "x"),
                ],
                vec![candidate("path:src", &["a", "b"], &[])],
            ),
        ];
        for (name, missions, expected) in cases {
            assert_eq!(find_candidates(&missions), expected, "{name}");
        }
    }

    #[test]
    fn a_slot_is_contended_by_another_missions_load_in_both_samples() {
        let at = Utc::now();
        let owners = |cwd: &Path| -> Vec<String> {
            match cwd.to_str().unwrap() {
                "/tmp/a" => vec!["a".into()],
                "/tmp/b" => vec!["b".into()],
                "/tmp/shared" => vec!["a".into(), "c".into()],
                _ => vec![],
            }
        };
        let busy = |pid: u32, cwd: &str, cpu: f32| Sampled {
            pid,
            cpu,
            command: "yes".into(),
            cwd: Some(PathBuf::from(cwd)),
        };
        let held = candidate("slot:benchmark", &["a"], &["b"]);
        let both = vec![
            busy(1, "/tmp/a", 99.0),
            busy(2, "/tmp/b", 30.0),
            busy(3, "/tmp/b", 40.0),
        ];
        let verdict = slot_verdict(&held, &owners, &both, &both, at);
        assert!(verdict.check.contended, "b sums 70% in both samples");
        assert_eq!(verdict.blocked.as_deref(), Some("a"));
        assert_eq!(
            verdict
                .check
                .uses
                .iter()
                .map(|u| (u.pid, u.task_id.as_deref()))
                .collect::<Vec<_>>(),
            [(3, Some("b")), (2, Some("b"))],
            "only the other mission's processes, busiest first"
        );
        let calm = vec![busy(1, "/tmp/a", 99.0), busy(2, "/tmp/b", 20.0)];
        assert!(
            !slot_verdict(&held, &owners, &both, &calm, at)
                .check
                .contended,
            "one busy sample is not enough"
        );
        assert!(
            !slot_verdict(&held, &owners, &calm, &both, at)
                .check
                .contended
        );
        let own = vec![busy(1, "/tmp/a", 300.0), busy(4, "/tmp/elsewhere", 300.0)];
        assert!(
            !slot_verdict(&held, &owners, &own, &own, at).check.contended,
            "the holder's own load and outside processes never count"
        );
        let shared = vec![busy(5, "/tmp/shared", 80.0)];
        let verdict = slot_verdict(
            &candidate("slot:benchmark", &["a"], &["c"]),
            &owners,
            &shared,
            &shared,
            at,
        );
        assert!(verdict.check.contended);
        assert_eq!(
            verdict.check.uses[0].task_id, None,
            "a shared checkout reads as a process in its path"
        );
        assert_eq!(verdict.loads[0].cwd.as_deref(), Some("/tmp/shared"));
    }
}
