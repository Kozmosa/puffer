use crate::handler::{handle_command, CommandOutcome, PLATFORM_ID};
use crate::MatrixConfig;
use anyhow::{Context, Result};
use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::{Client, Room, RoomState};
use puffer_connector_core::{Connector, ConnectorHandle, ConnectorRuntime, ConnectorStartError};
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Matrix connector ready to be started by the puffer connector hub.
pub struct MatrixConnector {
    config: MatrixConfig,
}

impl MatrixConnector {
    pub fn new(config: MatrixConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &MatrixConfig {
        &self.config
    }
}

impl Connector for MatrixConnector {
    fn id(&self) -> &str {
        PLATFORM_ID
    }

    fn start(
        self: Box<Self>,
        runtime: Arc<ConnectorRuntime>,
    ) -> Result<ConnectorHandle, ConnectorStartError> {
        if self.config.homeserver_url.trim().is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "homeserver_url is empty".to_string(),
            });
        }
        if self.config.username.trim().is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "username is empty".to_string(),
            });
        }
        if self.config.password.is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "password is empty".to_string(),
            });
        }

        // One-shot channel that fires a cooperative shutdown. The worker
        // tokio runtime picks it up and drops the sync task.
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        // Signal back from the worker thread once the Tokio runtime is
        // ready to be interrupted.
        let (ready_tx, ready_rx) = mpsc::channel::<()>();

        let config = self.config.clone();
        let join = std::thread::Builder::new()
            .name("puffer-connector-matrix".to_string())
            .spawn(move || -> Result<()> {
                let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .context("failed to build tokio runtime for matrix connector")?;
                let _ = ready_tx.send(());
                tokio_runtime.block_on(run_client(config, runtime, shutdown_rx))
            })
            .map_err(|error| ConnectorStartError::other(PLATFORM_ID, error.into()))?;

        ready_rx
            .recv()
            .map_err(|error| ConnectorStartError::other(PLATFORM_ID, error.into()))?;

        let shutdown: Box<dyn FnOnce() + Send> = Box::new(move || {
            let _ = shutdown_tx.send(());
        });

        Ok(ConnectorHandle {
            id: PLATFORM_ID.to_string(),
            shutdown,
            join,
        })
    }
}

async fn run_client(
    config: MatrixConfig,
    runtime: Arc<ConnectorRuntime>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let client = Client::builder()
        .homeserver_url(&config.homeserver_url)
        .build()
        .await
        .context("failed to build matrix client")?;

    client
        .matrix_auth()
        .login_username(&config.username, &config.password)
        .send()
        .await
        .context("failed to log in to matrix homeserver")?;

    // An initial sync populates the client store before we start
    // dispatching messages — without it, `get_room` returns None for
    // rooms the bot is in.
    client
        .sync_once(SyncSettings::default())
        .await
        .context("initial matrix sync failed")?;

    // Stash the config + runtime so the event handler can read them by
    // `Ctx` lookup rather than capturing them in the closure (which would
    // force a `Send + Sync + 'static` clone on every call).
    client.add_event_handler_context(HandlerCtx {
        config: Arc::new(config.clone()),
        runtime: runtime.clone(),
    });
    client.add_event_handler(on_room_message);

    // Drive the long-poll sync loop in a background task so we can race
    // it against the shutdown signal.
    let sync_client = client.clone();
    let sync_settings = SyncSettings::default();
    let sync_handle = tokio::spawn(async move {
        // `sync` only returns on error; we treat a clean shutdown as Ok.
        let _ = sync_client.sync(sync_settings).await;
    });

    let _ = shutdown_rx.await;
    sync_handle.abort();
    // `abort` drops the future, which in turn drops the sync stream. We
    // still await the handle so the thread's JoinHandle reflects the
    // final state accurately.
    let _ = sync_handle.await;
    Ok(())
}

/// Context passed into matrix-sdk event handlers via
/// [`Client::add_event_handler_context`].
#[derive(Clone)]
struct HandlerCtx {
    config: Arc<MatrixConfig>,
    runtime: Arc<ConnectorRuntime>,
}

async fn on_room_message(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    client: Client,
    ctx: matrix_sdk::event_handler::Ctx<HandlerCtx>,
) {
    // Only react inside rooms the bot has actually joined. Invites and
    // left rooms should stay silent until someone accepts on our behalf.
    if room.state() != RoomState::Joined {
        return;
    }

    // Ignore our own echoes.
    if let Some(me) = client.user_id() {
        if event.sender == me {
            return;
        }
    }

    let text = match &event.content.msgtype {
        MessageType::Text(content) => content.body.clone(),
        _ => return,
    };

    let sender = event.sender.to_string();
    let room_id = room.room_id().to_string();
    let runtime = ctx.runtime.clone();
    let config = ctx.config.clone();

    // Dispatch blocks on the shared runtime mutex; park it on a blocking
    // worker so we don't starve the tokio reactor.
    let outcome = tokio::task::spawn_blocking(move || {
        handle_command(&runtime, &room_id, Some(&sender), &text, &config)
    })
    .await;

    let reply = match outcome {
        Ok(Ok(CommandOutcome::Ignored)) => return,
        Ok(Ok(CommandOutcome::Reply(text))) => text,
        Ok(Ok(CommandOutcome::AgentReply { text, .. })) => text,
        Ok(Err(error)) => format!("Puffer error: {error}"),
        Err(join_error) => format!("Puffer error: {join_error}"),
    };

    let content = RoomMessageEventContent::text_plain(reply);
    let _ = room.send(content).await;
}
