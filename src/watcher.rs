use anyhow::Result;
use notify::{
    recommended_watcher, Event, RecommendedWatcher, RecursiveMode,
    Watcher as _Watcher,
};
use std::path::PathBuf;

pub struct Watcher {
    watcher: RecommendedWatcher,
    paths: Vec<PathBuf>,
}

impl Watcher {
    pub fn new(
        tx: tokio::sync::mpsc::Sender<Event>,
        paths: Vec<PathBuf>,
    ) -> Result<Self> {
        let watcher = recommended_watcher(move |res| {
            if let Ok(event) = res {
                tx.blocking_send(event).expect("send to channel");
            }
        })?;

        Ok(Self { watcher, paths })
    }

    pub fn watch_all(&mut self) -> Result<()> {
        for path in &self.paths {
            self.watcher.watch(path, RecursiveMode::NonRecursive)?;
        }
        Ok(())
    }

    pub fn unwatch_all(&mut self) -> Result<()> {
        for path in &self.paths {
            self.watcher.unwatch(path)?;
        }
        Ok(())
    }
}
