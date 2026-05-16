use std::{
    collections::{HashMap, HashSet},
    env,
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
    Trigger,
    Quit,
}

pub async fn run(config: Config, colors: Colors) -> Result<()> {
    let watch_paths = config.watch_paths();
    let config = Arc::new(config);
    let colors = Arc::new(colors);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let wx = make_watchexec(tx.clone(), watch_paths.clone())?;
    let mut fs_ready = wx.config.fs_ready();
    wx.config.pathset(parent_dirs(&watch_paths));
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
                    Some(Event::Trigger) => {
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
    let mut seen = HashSet::new();
    let mut dirs = Vec::new();
    for path in paths {
        let dir = path.parent().unwrap_or(path);
        if seen.insert(dir.to_path_buf()) {
            dirs.push(WatchedPath::non_recursive(dir));
        }
    }
    dirs
}

fn make_watchexec(
    tx: mpsc::UnboundedSender<Event>,
    watch_paths: Vec<PathBuf>,
) -> Result<Arc<Watchexec>> {
    let snapshots = Arc::new(Mutex::new(
        watch_paths
            .iter()
            .map(|path| (path.clone(), file_stat(path)))
            .collect::<HashMap<_, _>>(),
    ));
    let wx = Watchexec::new(move |mut action| {
        let path_changed = {
            let mut snapshots = snapshots.lock().expect("watch snapshots lock poisoned");
            action
                .paths()
                .any(|(path, _)| watched_path_changed(path, &watch_paths, &mut snapshots))
        };

        if action.signals().next().is_some() {
            let _ = tx.send(Event::Quit);
            action.quit();
            return action;
        }

        if path_changed {
            let _ = tx.send(Event::Trigger);
        }

        action
    })?;
    Ok(wx)
}

fn watched_path_changed(
    path: &Path,
    watch_paths: &[PathBuf],
    snapshots: &mut HashMap<PathBuf, Option<FileStat>>,
) -> bool {
    let Some(watched_path) = matching_watched_path(path, watch_paths) else {
        return false;
    };
    let current = file_stat(watched_path);
    let previous = snapshots.insert(watched_path.clone(), current);
    previous != Some(current)
}

fn matching_watched_path<'a>(path: &Path, watch_paths: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let canonical = absolute.canonicalize().ok();

    watch_paths.iter().find(|watched| {
        *watched == &absolute || canonical.as_ref().is_some_and(|path| *watched == path)
    })
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct FileStat {
    mtime: i64,
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
