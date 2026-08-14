//! TUI Bridge — manages stdio JSON connection and background task execution

use super::handlers::handle_request;
use super::protocol::{TuiEvent, TuiMessage, TuiRequest, TuiResponse};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Shared mutable state for the TUI adapter
#[derive(Default)]
pub struct TuiState {
    pub session_id: Mutex<Option<String>>,
    pub task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub cancel_token: Mutex<Option<crate::cancellation::CancellationToken>>,
}

/// Sender for pushing events to the TUI frontend
pub type EventSender = mpsc::UnboundedSender<TuiEvent>;

/// Callback type for emitting events from async task handlers
pub type EventCallback = Arc<dyn Fn(TuiEvent) -> Result<(), std::io::Error> + Send + Sync>;

/// Creates an event callback that writes to a channel
pub fn make_event_sender() -> (EventSender, EventCallback) {
    let (tx, _rx) = mpsc::unbounded_channel::<TuiEvent>();
    let tx_clone = tx.clone();
    let callback: EventCallback = Arc::new(move |event: TuiEvent| {
        tx_clone
            .send(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    });
    (tx, callback)
}

pub struct TuiBridge {
    stdin: tokio::io::Stdin,
    stdout: tokio::io::Stdout,
    state: Arc<TuiState>,
}

impl TuiBridge {
    pub fn new() -> Self {
        Self {
            stdin: tokio::io::stdin(),
            stdout: tokio::io::stdout(),
            state: Arc::new(TuiState::default()),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut reader = tokio::io::BufReader::new(self.stdin);
        let mut line = String::new();
        let state = self.state.clone();

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<TuiEvent>();

        loop {
            tokio::select! {
                biased;
                Some(event) = event_rx.recv() => {
                    if let Err(e) = self.stdout.write_all(format!("{}\n", serde_json::to_string(&event)?).as_bytes()).await {
                        eprintln!("TUI bridge event write error: {}", e);
                        break;
                    }
                    if let Err(e) = self.stdout.flush().await {
                        eprintln!("TUI bridge event flush error: {}", e);
                        break;
                    }
                }
                result = reader.read_line(&mut line) => {
                    match result {
                        Ok(0) => break,
                        Ok(_) => {
                            if let Some(msg) = TuiMessage::parse(&line) {
                                match msg {
                                    TuiMessage::Request(req) => {
                                        let resp = handle_request(req, &state, event_tx.clone()).await;
                                        if let Err(e) = self.stdout.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes()).await {
                                            eprintln!("TUI bridge write error: {}", e);
                                            break;
                                        }
                                        if let Err(e) = self.stdout.flush().await {
                                            eprintln!("TUI bridge flush error: {}", e);
                                            break;
                                        }
                                    }
                                    TuiMessage::Response(_) | TuiMessage::Event(_) => {}
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("TUI bridge read error: {}", e);
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
