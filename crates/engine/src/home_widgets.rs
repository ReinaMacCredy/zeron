use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use zeron_preview::discovery::{self, Listener};
use zeron_proto::{
    Chat, GitActivity, GitActivityDay, GitActivityProject, ListeningService, Space, SystemStats,
};

const GIT_TIMEOUT: Duration = Duration::from_secs(20);
const LOG_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RepoGroup {
    pub space_id: String,
    pub name: String,
    pub path: PathBuf,
}

pub(crate) fn repo_groups(spaces: &[Space], device_id: &str) -> Vec<RepoGroup> {
    let mut seen = HashSet::new();
    spaces
        .iter()
        .filter(|space| space.device_id == device_id && space.git_detected)
        .filter(|space| seen.insert(space.repository_id.clone().unwrap_or(space.path.clone())))
        .map(|space| RepoGroup {
            space_id: space.id.clone(),
            name: space.display_name().to_owned(),
            path: PathBuf::from(&space.path),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct CommitStat {
    hash: String,
    authored: DateTime<FixedOffset>,
    additions: u64,
    deletions: u64,
}

fn parse_log(stdout: &str) -> Vec<CommitStat> {
    stdout
        .split('\x1e')
        .filter_map(|record| {
            let mut lines = record.lines();
            let (hash, date) = lines.next()?.split_once('\x1f')?;
            let authored = DateTime::parse_from_rfc3339(date.trim()).ok()?;
            let (mut additions, mut deletions) = (0, 0);
            for line in lines {
                let mut parts = line.split('\t');
                if let (Some(add), Some(del), Some(_)) = (parts.next(), parts.next(), parts.next())
                {
                    additions += add.parse::<u64>().unwrap_or(0);
                    deletions += del.parse::<u64>().unwrap_or(0);
                }
            }
            Some(CommitStat {
                hash: hash.trim().to_owned(),
                authored,
                additions,
                deletions,
            })
        })
        .collect()
}

async fn repo_commits(path: &Path, since: NaiveDate) -> Vec<CommitStat> {
    let run = async {
        let email = crate::working_tree::git_with(path, &["config", "user.email"], 4096, true)
            .await
            .ok()?;
        let email = String::from_utf8_lossy(&email.stdout).trim().to_owned();
        if email.is_empty() {
            return None;
        }
        let author = format!("--author=<{email}>");
        let since = format!("--since={since} 00:00:00");
        let log = crate::working_tree::git_with(
            path,
            &[
                "log",
                "--branches",
                "--no-merges",
                "--fixed-strings",
                "--regexp-ignore-case",
                &author,
                &since,
                "--format=%x1e%H%x1f%aI",
                "--numstat",
            ],
            LOG_MAX_BYTES,
            false,
        )
        .await
        .ok()?;
        Some(parse_log(&String::from_utf8_lossy(&log.stdout)))
    };
    tokio::time::timeout(GIT_TIMEOUT, run)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub(crate) async fn git_activity(
    groups: Vec<RepoGroup>,
    filter: Option<&str>,
    days: u32,
    today: NaiveDate,
) -> GitActivity {
    let days = days.clamp(1, 90);
    let first = today - chrono::Days::new(u64::from(days - 1));
    let logs =
        futures::future::join_all(groups.iter().map(|group| repo_commits(&group.path, first)))
            .await;
    let in_range = |commit: &CommitStat| {
        let date = commit.authored.with_timezone(&Local).date_naive();
        (first..=today).contains(&date).then_some(date)
    };
    let mut seen = HashSet::new();
    let mut per_day: HashMap<NaiveDate, u32> = HashMap::new();
    let mut activity = GitActivity::default();
    for (group, commits) in groups.iter().zip(logs) {
        let mut own = HashSet::new();
        for commit in commits {
            let Some(date) = in_range(&commit) else {
                continue;
            };
            own.insert(commit.hash.clone());
            let counted = filter.is_none_or(|id| id == group.space_id);
            if counted && seen.insert(commit.hash.clone()) {
                *per_day.entry(date).or_default() += 1;
                activity.commits += 1;
                activity.additions += commit.additions;
                activity.deletions += commit.deletions;
            }
        }
        activity.projects.push(GitActivityProject {
            space_id: group.space_id.clone(),
            name: group.name.clone(),
            commits: own.len() as u32,
        });
    }
    activity.days = first
        .iter_days()
        .take(days as usize)
        .map(|date| GitActivityDay {
            date,
            commits: per_day.get(&date).copied().unwrap_or(0),
        })
        .collect();
    activity
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectRoot {
    path: PathBuf,
    project: String,
    worktree: Option<String>,
}

pub(crate) fn project_roots(spaces: &[Space], chats: &[Chat], device_id: &str) -> Vec<ProjectRoot> {
    let canonical =
        |path: &str| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let local: Vec<&Space> = spaces.iter().filter(|s| s.device_id == device_id).collect();
    let mut roots: Vec<ProjectRoot> = local
        .iter()
        .map(|space| ProjectRoot {
            path: canonical(&space.path),
            project: space.display_name().to_owned(),
            worktree: None,
        })
        .collect();
    for chat in chats.iter().filter(|c| c.device_id == device_id) {
        let Some(cwd) = chat.cwd.as_deref() else {
            continue;
        };
        let path = canonical(cwd);
        if roots.iter().any(|root| root.path == path) {
            continue;
        }
        let Some(space) = chat
            .space_id
            .as_deref()
            .and_then(|id| local.iter().find(|s| s.id == id))
        else {
            continue;
        };
        roots.push(ProjectRoot {
            worktree: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            path,
            project: space.display_name().to_owned(),
        });
    }
    roots.sort_by_key(|root| std::cmp::Reverse(root.path.components().count()));
    roots
}

pub(crate) fn services(
    listeners: Vec<Listener>,
    roots: &[ProjectRoot],
    all_ports: bool,
) -> Vec<ListeningService> {
    let mut seen = HashSet::new();
    let mut services: Vec<ListeningService> = listeners
        .into_iter()
        .filter(|l| seen.insert((l.pid, l.address.port())))
        .filter_map(|listener| {
            let root = roots.iter().find(|root| listener.belongs_to(&root.path));
            if root.is_none() && !all_ports {
                return None;
            }
            let (framework, rank) = discovery::framework(&listener.args);
            Some(ListeningService {
                port: listener.address.port(),
                pid: listener.pid,
                command: listener
                    .args
                    .first()
                    .and_then(|arg| Path::new(arg).file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                framework: (rank >= 80).then(|| framework.to_owned()),
                project: root.map(|r| r.project.clone()),
                worktree: root.and_then(|r| r.worktree.clone()),
            })
        })
        .collect();
    services.sort_by_key(|s| (s.project.is_none(), s.port));
    services
}

pub(crate) async fn listening_services(
    roots: Vec<ProjectRoot>,
    all_ports: bool,
) -> Vec<ListeningService> {
    let listeners = tokio::task::spawn_blocking(|| {
        discovery::listeners(&mut discovery::Origins::default()).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    services(listeners, &roots, all_ports)
}

pub(crate) async fn system_stats() -> SystemStats {
    tokio::task::spawn_blocking(machine::stats)
        .await
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod machine {
    use std::ffi::CString;
    use std::sync::Mutex;

    use zeron_proto::SystemStats;

    static LAST_TICKS: Mutex<Option<[u64; 4]>> = Mutex::new(None);

    #[allow(deprecated)]
    fn host() -> libc::mach_port_t {
        unsafe { libc::mach_host_self() }
    }

    fn cpu_ticks() -> Option<[u64; 4]> {
        let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let status = unsafe {
            libc::host_statistics(
                host(),
                libc::HOST_CPU_LOAD_INFO,
                &mut info as *mut _ as libc::host_info_t,
                &mut count,
            )
        };
        (status == libc::KERN_SUCCESS).then(|| info.cpu_ticks.map(u64::from))
    }

    fn busy_percent(before: [u64; 4], after: [u64; 4]) -> f32 {
        let delta: Vec<u64> = after
            .iter()
            .zip(before)
            .map(|(a, b)| a.saturating_sub(b))
            .collect();
        let total: u64 = delta.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let idle = delta[libc::CPU_STATE_IDLE as usize];
        (total - idle) as f32 / total as f32 * 100.0
    }

    fn cpu_percent() -> f32 {
        let Some(now) = cpu_ticks() else {
            return 0.0;
        };
        let mut last = LAST_TICKS.lock().unwrap_or_else(|e| e.into_inner());
        let before = match *last {
            Some(before) => before,
            None => {
                drop(last);
                std::thread::sleep(std::time::Duration::from_millis(250));
                let later = cpu_ticks().unwrap_or(now);
                last = LAST_TICKS.lock().unwrap_or_else(|e| e.into_inner());
                *last = Some(later);
                return busy_percent(now, later);
            }
        };
        *last = Some(now);
        busy_percent(before, now)
    }

    fn sysctl_u64(name: &str) -> Option<u64> {
        let name = CString::new(name).ok()?;
        let mut value: u64 = 0;
        let mut size = std::mem::size_of::<u64>();
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut value as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0).then_some(match size {
            4 => value & 0xffff_ffff,
            _ => value,
        })
    }

    fn memory_used() -> Option<u64> {
        let mut info: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let status = unsafe {
            libc::host_statistics64(
                host(),
                libc::HOST_VM_INFO64,
                &mut info as *mut _ as libc::host_info64_t,
                &mut count,
            )
        };
        if status != libc::KERN_SUCCESS {
            return None;
        }
        let page = sysctl_u64("vm.pagesize")?;
        let app =
            u64::from(info.internal_page_count).saturating_sub(u64::from(info.purgeable_count));
        Some((app + u64::from(info.wire_count) + u64::from(info.compressor_page_count)) * page)
    }

    fn disk() -> Option<(u64, u64)> {
        ["/System/Volumes/Data", "/"].iter().find_map(|path| {
            let path = CString::new(*path).ok()?;
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
                return None;
            }
            let block = stat.f_frsize as u64;
            let total = stat.f_blocks as u64 * block;
            let available = stat.f_bavail as u64 * block;
            Some((total.saturating_sub(available), total))
        })
    }

    pub(super) fn stats() -> SystemStats {
        let (disk_used, disk_total) = disk().unwrap_or_default();
        let memory_total = sysctl_u64("hw.memsize").unwrap_or_default();
        SystemStats {
            available: memory_total > 0,
            cores: sysctl_u64("hw.ncpu").unwrap_or_default() as u32,
            cpu_percent: cpu_percent(),
            memory_used: memory_used().unwrap_or_default().min(memory_total),
            memory_total,
            disk_used,
            disk_total,
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn this_mac_reports_plausible_numbers() {
            let stats = super::stats();
            assert!(stats.available);
            assert!(stats.cores > 0);
            assert!(stats.memory_used > 0 && stats.memory_used <= stats.memory_total);
            assert!(stats.disk_used > 0 && stats.disk_used <= stats.disk_total);
            assert!((0.0..=100.0).contains(&stats.cpu_percent));
        }

        #[test]
        fn busy_share_excludes_idle_ticks() {
            let idle = libc::CPU_STATE_IDLE as usize;
            let mut after = [10, 10, 0, 0];
            after[idle] = 30;
            assert_eq!(super::busy_percent([0; 4], after), 40.0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod machine {
    pub(super) fn stats() -> zeron_proto::SystemStats {
        zeron_proto::SystemStats::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit_messages::tests::git;
    use std::net::{Ipv4Addr, SocketAddr};

    fn space(id: &str, path: &Path, repository: Option<&str>) -> Space {
        Space {
            id: id.into(),
            device_id: "dev".into(),
            path: path.to_string_lossy().into_owned(),
            name: None,
            git_detected: true,
            git_checked_at: None,
            checkout_id: None,
            repository_id: repository.map(Into::into),
            created_at: chrono::Utc::now(),
            github_repo: Default::default(),
        }
    }

    fn commit(repo: &Path, file: &str, lines: usize, email: &str, date: &str) {
        std::fs::write(repo.join(file), "x\n".repeat(lines)).unwrap();
        git(repo, &["add", "."]);
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-qm", file])
            .env("GIT_AUTHOR_NAME", "Someone")
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", "Someone")
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn log_records_sum_numstat_and_skip_binary_counts() {
        let out = "\x1eabc\x1f2026-10-01T12:00:00+07:00\n\n3\t1\ta.rs\n-\t-\timg.png\n\x1edef\x1f2026-10-02T09:00:00+00:00\n";
        let commits = parse_log(out);
        assert_eq!(commits.len(), 2);
        assert_eq!((commits[0].additions, commits[0].deletions), (3, 1));
        assert_eq!(commits[1].hash, "def");
        assert_eq!(commits[1].additions, 0);
    }

    #[tokio::test]
    async fn activity_counts_own_commits_per_day_and_dedupes_clones() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "Me@Example.com"]);
        commit(&repo, "d", 1, "me@example.com", "2026-09-01T12:00:00");
        commit(&repo, "a", 3, "me@example.com", "2026-10-05T12:00:00");
        commit(&repo, "b", 2, "other@example.com", "2026-10-05T13:00:00");
        commit(&repo, "c", 5, "me@example.com", "2026-10-06T12:00:00");
        let clone = dir.path().join("clone");
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                repo.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        git(&clone, &["config", "user.email", "me@example.com"]);
        git(&clone, &["checkout", "-q", "-b", "main-copy"]);

        let today = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let groups = vec![
            RepoGroup {
                space_id: "s1".into(),
                name: "repo".into(),
                path: repo.clone(),
            },
            RepoGroup {
                space_id: "s2".into(),
                name: "clone".into(),
                path: clone.clone(),
            },
        ];
        let all = git_activity(groups.clone(), None, 14, today).await;
        assert_eq!(all.commits, 2);
        assert_eq!((all.additions, all.deletions), (8, 0));
        assert_eq!(all.days.len(), 14);
        assert_eq!(all.days.last().unwrap().commits, 1);
        assert_eq!(all.days[12].commits, 1);
        assert_eq!(
            all.projects.iter().map(|p| p.commits).collect::<Vec<_>>(),
            [2, 2]
        );

        let filtered = git_activity(groups, Some("s2"), 14, today).await;
        assert_eq!(filtered.commits, 2);
    }

    #[test]
    fn groups_merge_spaces_of_one_repository() {
        let spaces = vec![
            space("a", Path::new("/r/a"), Some("commit:1")),
            space("b", Path::new("/r/b"), Some("commit:1")),
            space("c", Path::new("/r/c"), None),
        ];
        let groups = repo_groups(&spaces, "dev");
        assert_eq!(
            groups
                .iter()
                .map(|g| g.space_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "c"]
        );
    }

    fn listener(pid: u32, port: u16, cwd: &str, args: &[&str]) -> Listener {
        Listener {
            pid,
            parent: 1,
            cwd: PathBuf::from(cwd),
            args: args.iter().map(|s| s.to_string()).collect(),
            started_at: 0,
            address: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
            zeron_owned: false,
            origin: None,
            chat_id: None,
        }
    }

    #[test]
    fn services_label_project_and_worktree_and_filter_by_mode() {
        let roots = vec![
            ProjectRoot {
                path: PathBuf::from("/code/shop/.wt/checkout-fix"),
                project: "shop".into(),
                worktree: Some("checkout-fix".into()),
            },
            ProjectRoot {
                path: PathBuf::from("/code/shop"),
                project: "shop".into(),
                worktree: None,
            },
        ];
        let listeners = vec![
            listener(
                10,
                3001,
                "/code/shop/.wt/checkout-fix",
                &["/usr/bin/node", "next", "dev"],
            ),
            listener(
                10,
                3001,
                "/code/shop/.wt/checkout-fix",
                &["/usr/bin/node", "next", "dev"],
            ),
            listener(11, 3000, "/code/shop/web", &["/opt/bin/node", "server.js"]),
            listener(12, 5432, "/usr/local/var", &["/opt/bin/postgres"]),
        ];
        let zeron = services(listeners.clone(), &roots, false);
        assert_eq!(
            zeron.iter().map(|s| s.port).collect::<Vec<_>>(),
            [3000, 3001]
        );
        assert_eq!(zeron[1].worktree.as_deref(), Some("checkout-fix"));
        assert_eq!(zeron[1].framework.as_deref(), Some("Next.js"));
        assert_eq!(zeron[0].command, "node");
        assert_eq!(zeron[0].worktree, None);

        let all = services(listeners, &roots, true);
        assert_eq!(
            all.iter().map(|s| s.port).collect::<Vec<_>>(),
            [3000, 3001, 5432]
        );
        assert_eq!(all[2].project, None);
        assert_eq!(all[2].command, "postgres");
    }
}
