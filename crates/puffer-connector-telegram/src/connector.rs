use crate::handler::{handle_command, CommandOutcome, PLATFORM_ID};
use crate::TelegramConfig;
use anyhow::{Context, Result};
use puffer_connector_core::{
    Connector, ConnectorHandle, ConnectorRuntime, ConnectorStartError,
};
use std::sync::mpsc;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatId;
use tokio::sync::oneshot;

/// Telegram connector ready to be started by the puffer connector hub.
pub struct TelegramConnector {
    config: TelegramConfig,
}

impl TelegramConnector {
    pub fn new(config: TelegramConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &TelegramConfig {
        &self.config
    }
}

impl Connector for TelegramConnector {
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

        // One-shot channel that fires teloxide's cooperative shutdown.
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        // Signal back from the worker thread once the Tokio runtime is
        // ready to be interrupted.
        let (ready_tx, ready_rx) = mpsc::channel::<()>();

        let config = self.config.clone();
        let join = std::thread::Builder::new()
            .name("puffer-connector-telegram".to_string())
            .spawn(move || -> Result<()> {
                let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .context("failed to build tokio runtime for telegram connector")?;
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
    config: TelegramConfig,
    runtime: Arc<ConnectorRuntime>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let bot = Bot::new(config.token.clone());

    let runtime_for_handler = runtime.clone();
    let config_for_handler = config.clone();

    let handler = Update::filter_message().endpoint(
        move |bot: Bot, message: Message| {
            let runtime = runtime_for_handler.clone();
            let config = config_for_handler.clone();
            async move { process_message(bot, message, runtime, config).await }
        },
    );

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build();

    let shutdown_token = dispatcher.shutdown_token();
    tokio::spawn(async move {
        if shutdown_rx.await.is_ok() {
            let _ = shutdown_token.shutdown();
        }
    });

    dispatcher.dispatch().await;
    Ok(())
}

async fn process_message(
    bot: Bot,
    message: Message,
    runtime: Arc<ConnectorRuntime>,
    config: TelegramConfig,
) -> Result<(), teloxide::RequestError> {
    let chat_id = message.chat.id.0;
    let user_id = message.from.as_ref().map(|user| user.id.0 as i64);
    let Some(text) = message.text().map(str::to_string) else {
        return Ok(());
    };

    // The dispatch itself blocks on the shared runtime mutex. Park on a
    // blocking worker so we don't starve the tokio reactor.
    let handler_runtime = runtime.clone();
    let handler_config = config.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        handle_command(&handler_runtime, chat_id, user_id, &text, &handler_config)
    })
    .await
    .map_err(|error| {
        teloxide::RequestError::from(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            error.to_string(),
        )))
    })?;

    let reply = match outcome {
        Ok(CommandOutcome::Ignored) => return Ok(()),
        Ok(CommandOutcome::Reply(text)) => text,
        Ok(CommandOutcome::AgentReply { text, .. }) => text,
        Err(error) => format!("Puffer error: {error}"),
    };

    bot.send_message(ChatId(chat_id), reply).await?;
    Ok(())
}
