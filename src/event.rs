use crate::diff::{DiffContent, DiffRequest};
use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Resize,
    FsChange,
    Tick,
    DiffLoaded {
        request: DiffRequest,
        result: Result<DiffContent, String>,
    },
}

pub fn spawn_event_producer(
    tx: mpsc::UnboundedSender<Event>,
    shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || loop {
        if *shutdown.borrow() {
            break;
        }

        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(CrosstermEvent::Key(key)) => {
                    if tx.send(Event::Key(key)).is_err() {
                        break;
                    }
                }
                Ok(CrosstermEvent::Resize(_, _)) => {
                    if tx.send(Event::Resize).is_err() {
                        break;
                    }
                }
                _ => {}
            }
        }
    })
}

pub fn spawn_tick_producer(
    tx: mpsc::UnboundedSender<Event>,
    shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut shutdown = shutdown;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if tx.send(Event::Tick).is_err() {
                        break;
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    })
}

pub fn spawn_watcher(
    tx: mpsc::UnboundedSender<Event>,
    watch_paths: Vec<std::path::PathBuf>,
    shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc as std_mpsc;

    tokio::task::spawn_blocking(move || {
        let (notify_tx, notify_rx) = std_mpsc::channel();
        let Ok(mut watcher) = RecommendedWatcher::new(notify_tx, Config::default()) else {
            return;
        };

        for path in &watch_paths {
            if path.exists() {
                let _ = watcher.watch(path, RecursiveMode::NonRecursive);
            }
        }

        loop {
            if *shutdown.borrow() {
                break;
            }

            match notify_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(_) => {
                    if tx.send(Event::FsChange).is_err() {
                        break;
                    }
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
    })
}
