//! Watches the music, film and TV folders. A new rip, a retag or a deleted album is picked up
//! a couple of seconds after it happens, by rescanning just the folders touched.

use crate::Shared;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub fn start(state: Shared) {
    let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();
    let root = state.music_root.clone();

    // notify runs its own thread and calls us back; we only forward the paths.
    let mut watcher: RecommendedWatcher =
        match notify::recommended_watcher(move |res: notify::Result<Event>| match res {
            // Only real changes matter; reading a file for its tags is not one.
            Ok(ev)
                if matches!(
                    ev.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) =>
            {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
            Ok(_) => {}
            Err(e) => warn!("watch: {e}"),
        }) {
            Ok(w) => w,
            Err(e) => {
                error!("could not start the folder watcher: {e}");
                return;
            }
        };
    if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
        error!("could not watch {}: {e}", root.display());
        return;
    }
    info!("watching {}", root.display());
    for dir in ["Movies", "TV-Shows", "Books", "Comics"] {
        let d = state.args.library.join(dir);
        if d.is_dir() {
            match watcher.watch(&d, RecursiveMode::Recursive) {
                Ok(()) => info!("watching {}", d.display()),
                Err(e) => warn!("could not watch {}: {e}", d.display()),
            }
        }
    }

    tokio::spawn(async move {
        let _keep = watcher; // dropped when the task ends
        loop {
            let Some(first) = rx.recv().await else { break };
            let mut dirs: HashSet<PathBuf> = HashSet::new();
            dirs.insert(dir_of(&first));
            // Collect everything that arrives in the next two seconds, then scan once.
            let deadline = tokio::time::sleep(Duration::from_secs(2));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    more = rx.recv() => match more {
                        Some(p) => { dirs.insert(dir_of(&p)); }
                        None => break,
                    }
                }
            }
            // Keep only the outermost folders, so nothing is walked twice.
            let all: Vec<PathBuf> = dirs.into_iter().filter(|d| d.exists()).collect();
            let dirs: Vec<PathBuf> = all
                .iter()
                .filter(|d| !all.iter().any(|o| o != *d && d.starts_with(o)))
                .cloned()
                .collect();
            if dirs.is_empty() {
                // Folders were removed: a full scan prunes what is gone.
                if let Err(e) = crate::run_scan(&state).await {
                    warn!("rescan after removal: {e:#}");
                }
                continue;
            }
            info!("changes in {} folder(s), rescanning", dirs.len());
            match crate::scan_paths(&state, Some(dirs)).await {
                Ok(_) if state.args.auto_fetch && !state.args.no_network => {
                    if let Err(e) = crate::fetch_missing(&state, crate::FetchKind::All, false).await
                    {
                        warn!("fetch after rescan: {e:#}");
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("rescan: {e:#}"),
            }
        }
    });
}

fn dir_of(p: &std::path::Path) -> PathBuf {
    if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| p.to_path_buf())
    }
}
