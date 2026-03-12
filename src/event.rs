use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Resize,
    FsChange,
    Tick,
}

pub fn spawn_event_producer(tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
        loop {
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                match event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        let _ = tx.send(Event::Key(key));
                    }
                    Ok(CrosstermEvent::Resize(_, _)) => {
                        let _ = tx.send(Event::Resize);
                    }
                    _ => {}
                }
            }
        }
    });
}

pub fn spawn_tick_producer(tx: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = tx.send(Event::Tick);
        }
    });
}

pub fn spawn_watcher(tx: mpsc::UnboundedSender<Event>, watch_paths: Vec<std::path::PathBuf>) {
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc as std_mpsc;

    tokio::spawn(async move {
        let (notify_tx, notify_rx) = std_mpsc::channel();
        let mut watcher =
            RecommendedWatcher::new(notify_tx, Config::default()).unwrap_or_else(|e| {
                panic!("Failed to create watcher: {e}");
            });

        for path in &watch_paths {
            if path.exists() {
                let _ = watcher.watch(path, RecursiveMode::NonRecursive);
            }
        }

        loop {
            match notify_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(_) => {
                    let _ = tx.send(Event::FsChange);
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            tokio::task::yield_now().await;
        }
    });
}
