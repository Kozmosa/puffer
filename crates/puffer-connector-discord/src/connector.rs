use crate::handler::{handle_command, CommandOutcome, PLATFORM_ID};
use crate::DiscordConfig;
use anyhow::{Context as AnyhowContext, Result};
use puffer_connector_core::{
    Connector, ConnectorHandle, ConnectorRuntime, ConnectorStartError,
};
use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::GatewayIntents;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Discord connector ready to be started by the puffer connector hub.
pub struct DiscordConnector {
    config: DiscordConfig,
}

impl DiscordConnector {
    pub fn new(config: DiscordConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &DiscordConfig {
        &self.config
    }
}

impl Connector for DiscordConnector {
    fn id(&self) -> &str {
        PLATFORM_ID
    }

    fn start(
        self: Box<Self>,
        runtime: Arc<ConnectorRuntime>,
    ) -> Result<ConnectorHandle, ConnectorStartError> {
        if self.config.token.trim().is_empty() {
            return Err(ConnectorStartError::MissingConfig {
                id: PLATFORM_ID.to_string(),
                detail: "bot token is empty".to_string(),
            });
        }

        // One-shot channel that fires serenity's cooperative shutdown.
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        // Signal back from the worker thread once the Tokio runtime is
        // ready to be interrupted.
        let (ready_tx, ready_rx) = mpsc::channel::<()>();

        let config = self.config.clone();
        let join = std::thread::Builder::new()
            .name("puffer-connector-discord".to_string())
            .spawn(move || -> Result<()> {
                let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .context("failed to build tokio runtime for discord connector")?;
                let _ = ready_tx.send(());
                tokio_runtime.block_on(run_dispatcher(config, runtime, shutdown_rx))
            })
            .map_err(|error| ConnectorStartError::other(PLATFORM_ID, error.into()))?;

        // Wait for the worker to stand up before returning. `recv` times
        // out on thread death, which we surface as a start error.
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

async fn run_dispatcher(
    config: DiscordConfig,
    runtime: Arc<ConnectorRuntime>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = Handler {
        runtime: runtime.clone(),
        config: config.clone(),
    };

    let mut client = Client::builder(&config.token, intents)
        .event_handler(handler)
        .await
        .context("failed to build discord client")?;

    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        if shutdown_rx.await.is_ok() {
            shard_manager.shutdown_all().await;
        }
    });

    if let Err(error) = client.start().await {
        return Err(anyhow::anyhow!("discord client error: {error}"));
    }
    Ok(())
}

struct Handler {
    runtime: Arc<ConnectorRuntime>,
    config: DiscordConfig,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, message: Message) {
        // Ignore our own messages and other bots.
        if message.author.bot {
            return;
        }
        let channel_id = message.channel_id.get();
        let user_id = Some(message.author.id.get());
        let text = message.content.clone();

        let runtime = self.runtime.clone();
        let config = self.config.clone();

        // The dispatch itself blocks on the shared runtime mutex. Park on
        // a blocking worker so we don't starve the tokio reactor.
        let outcome = tokio::task::spawn_blocking(move || {
            handle_command(&runtime, channel_id, user_id, &text, &config)
        })
        .await;

        let reply = match outcome {
            Ok(Ok(CommandOutcome::Ignored)) => return,
            Ok(Ok(CommandOutcome::Reply(text))) => text,
            Ok(Ok(CommandOutcome::AgentReply { text, .. })) => text,
            Ok(Err(error)) => format!("Puffer error: {error}"),
            Err(error) => format!("Puffer error: {error}"),
        };

        if let Err(error) = message.channel_id.say(&ctx.http, reply).await {
            eprintln!("discord connector: failed to send reply: {error}");
        }
    }

    async fn ready(&self, _: Context, _ready: Ready) {
        // Intentionally silent — the connector hub logs start/stop.
    }
}
