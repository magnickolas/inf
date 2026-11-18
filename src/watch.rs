use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use watchexec::{Watchexec, sources::fs::WatchedPath};

use crate::{
    command::{self, RunResult},
    config::Config,
    output::Colors,
};

enum Event {
    Trigger(Vec<WatchedPath>),
    Quit,
}

pub(crate) async fn run(config: Config, colors: Colors) -> Result<()> {
    let watch_paths = config.watch_paths();
    let config = Arc::new(config);
    let colors = Arc::new(colors);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut watched_dirs = parent_dirs(&watch_paths);
    let wx = make_watchexec(tx, watch_paths, watched_dirs.clone())?;
    let mut fs_ready = wx.config.fs_ready();
    wx.config.pathset(watched_dirs.clone());
    let mut main = wx.main();
    let _ = fs_ready.changed().await;

    let mut active: Option<ActiveRun> = None;
    let mut pending = !config.postpone;

    loop {
        if active.is_none() && pending {
            active = Some(ActiveRun::spawn(Arc::clone(&config), Arc::clone(&colors)));
            pending = false;
        }

        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(Event::Trigger(dirs)) => {
                        if dirs != watched_dirs {
                            wx.config.pathset(dirs.clone());
                            let _ = fs_ready.changed().await;
                            watched_dirs = dirs;
                        }
                        pending = true;
                        if config.refresh && let Some(active) = &mut active {
                           active.cancel();
                        }
                    }
                    Some(Event::Quit) | None => {
                        if let Some(mut active) = active {
                            active.cancel();
                            let _ = active.join.await;
                        }
                        break;
                    }
                }
            }
            result = &mut main => {
                result.context("watchexec task failed")??;
                if let Some(mut active) = active {
                    active.cancel();
                    let _ = active.join.await;
                }
                break;
            }
            result = async {
                match active.as_mut() {
                    Some(active) => Some((&mut active.join).await),
                    None => None,
                }
            }, if active.is_some() => {
                active = None;
                match result {
                    Some(Ok(run_result)) => {
                        match run_result? {
                            RunResult::Cancelled if config.refresh => {
                                pending = true;
                            }
                            RunResult::Cancelled | RunResult::Completed => {}
                        }
                    }
                    Some(Err(err)) => return Err(err).context("run task failed"),
                    None => {}
                }
            }
        }
    }

    main.abort();
    Ok(())
}

fn parent_dirs(paths: &[PathBuf]) -> Vec<WatchedPath> {
    let mut seen_dirs = HashSet::new();
    let mut dirs = Vec::new();
    for path in paths {
        collect_watch_dirs(path, &mut HashSet::new(), &mut seen_dirs, &mut dirs);
    }
    dirs
}

fn collect_watch_dirs(
    path: &Path,
    seen_links: &mut HashSet<PathBuf>,
    seen_dirs: &mut HashSet<PathBuf>,
    dirs: &mut Vec<WatchedPath>,
) {
    if let Some(dir) = path
        .ancestors()
        .skip(1)
        .find_map(|parent| parent.canonicalize().ok())
        && seen_dirs.insert(dir.clone())
    {
        dirs.push(WatchedPath::non_recursive(dir));
    }

    // Watch the directory containing each symlink so replacements are visible,
    // and follow its target even while the final file is absent.
    let ancestors: Vec<_> = path.ancestors().collect();
    for component in ancestors.into_iter().rev() {
        if let Some(parent) = component.parent()
            && let Some(name) = component.file_name()
            && let Ok(target) = component.read_link()
            && let Ok(parent) = parent.canonicalize()
            && let Ok(suffix) = path.strip_prefix(component)
        {
            if !seen_links.insert(parent.join(name)) {
                break;
            }
            let mut target = parent.join(target);
            if !suffix.as_os_str().is_empty() {
                target.push(suffix);
            }
            if seen_dirs.insert(parent.clone()) {
                dirs.push(WatchedPath::non_recursive(parent));
            }
            collect_watch_dirs(&target, seen_links, seen_dirs, dirs);
            break;
        }
    }
}

fn make_watchexec(
    tx: mpsc::UnboundedSender<Event>,
    watch_paths: Vec<PathBuf>,
    watched_dirs: Vec<WatchedPath>,
) -> Result<Arc<Watchexec>> {
    let snapshots = watch_paths
        .iter()
        .map(|path| (path.clone(), file_stat(path)))
        .collect::<HashMap<_, _>>();
    let state = Mutex::new((snapshots, watched_dirs));
    let wx = Watchexec::new(move |mut action| {
        if action.signals().next().is_some() {
            let _ = tx.send(Event::Quit);
            action.quit();
            return action;
        }

        if action.paths().next().is_some() {
            let mut state = state.lock().expect("watch state lock poisoned");
            let (snapshots, watched_dirs) = &mut *state;
            // Refresh every snapshot, including symlink targets that disappeared.
            let path_changed = watch_paths.iter().fold(false, |changed, path| {
                watched_path_changed(path, snapshots) | changed
            });
            let dirs = parent_dirs(&watch_paths);
            if path_changed || dirs != *watched_dirs {
                *watched_dirs = dirs.clone();
                let _ = tx.send(Event::Trigger(dirs));
            }
        }

        action
    })?;
    Ok(wx)
}

fn watched_path_changed(
    watched_path: &Path,
    snapshots: &mut HashMap<PathBuf, Option<FileStat>>,
) -> bool {
    let current = file_stat(watched_path);
    let previous = snapshots.insert(watched_path.to_path_buf(), current);
    previous != Some(current)
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct FileStat {
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
    size: u64,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}

fn file_stat(path: &Path) -> Option<FileStat> {
    let meta = path.metadata().ok()?;
    Some(FileStat {
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        ctime: meta.ctime(),
        ctime_nsec: meta.ctime_nsec(),
        size: meta.size(),
        ino: meta.ino(),
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
    })
}

struct ActiveRun {
    join: JoinHandle<Result<RunResult>>,
    cancel_tx: watch::Sender<u64>,
    cancel_version: u64,
}

impl ActiveRun {
    fn spawn(config: Arc<Config>, colors: Arc<Colors>) -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(0);
        let join = tokio::spawn(command::compile_and_run(config, colors, cancel_rx));
        Self {
            join,
            cancel_tx,
            cancel_version: 0,
        }
    }

    fn cancel(&mut self) {
        self.cancel_version += 1;
        let _ = self.cancel_tx.send(self.cancel_version);
    }
}
