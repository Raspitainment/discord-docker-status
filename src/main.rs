#![warn(clippy::all)]

use anyhow::Context;
use bollard::{
    container::{LogOutput, LogsOptions},
    Docker,
};
use futures::TryStreamExt;
use itertools::Itertools;
use log::{error, info, warn};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use twilight_http::Client as HttpClient;
use twilight_model::{
    channel::message::{embed::EmbedFooter, Embed},
    id::{
        marker::{ChannelMarker, GuildMarker},
        Id,
    },
};

const GUILD: Id<GuildMarker> = Id::new(1209473653759016990);
const CATEGORY: Id<ChannelMarker> = Id::new(1218191011348615240);

#[tokio::main]
async fn main() {
    simplelog::TermLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )
    .unwrap();

    let containers = Arc::new(Mutex::new(HashMap::new()));

    let _containers = containers.clone();

    if let Err(e) = tokio::try_join!(container_thread(_containers), message_update(containers)) {
        error!("Error: {:#}", e);
    }
}

struct Container {
    id: String,
    name: String,
    image: String,
    command: String,
    status: String,
    logs: Vec<LogOutput>,
}

async fn container_thread(store: Arc<Mutex<HashMap<String, Container>>>) -> anyhow::Result<()> {
    let docker = Docker::connect_with_socket_defaults().context("Failed to connect to Docker")?;

    loop {
        info!("Updating containers");

        let containers = docker
            .list_containers::<&str>(None)
            .await
            .context("Failed to list containers")?;

        for container in containers {
            let x = [
                container.id.clone(),
                container
                    .names
                    .as_ref()
                    .and_then(|names| names.first().cloned()),
                container.image.clone(),
                container.command.clone(),
                container.status.clone(),
            ]
            .into_iter()
            .collect::<Option<Vec<_>>>();

            let Some([id, name, image, command, status]) = x.as_deref() else {
                warn!("Container has missing fields: {:?}", container);
                continue;
            };

            info!("Getting logs for container: {}", id);

            let logs = docker
                .logs::<&str>(
                    id,
                    Some(LogsOptions {
                        follow: false,
                        stdout: true,
                        stderr: true,
                        since: 0,
                        until: 0,
                        timestamps: false,
                        tail: "20",
                    }),
                )
                .try_collect::<Vec<_>>()
                .await
                .context("Failed to get logs")?;

            info!("Got {} logs", logs.len());

            store
                .lock()
                .await
                .entry(id.to_string())
                .or_insert(Container {
                    id: id.clone(),
                    name: name.clone(),
                    image: image.clone(),
                    command: command.clone(),
                    status: status.clone(),
                    logs: vec![],
                })
                .logs = logs;
        }

        info!("Updated containers");

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

async fn message_update(store: Arc<Mutex<HashMap<String, Container>>>) -> anyhow::Result<()> {
    let token = std::env::var("DISCORD_TOKEN").context("Failed to get DISCORD_TOKEN")?;

    let http = Arc::new(HttpClient::new(token));

    let mut channels = HashMap::<String, _>::new();

    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    loop {
        info!("Updating messages");

        for container in store.lock().await.values().collect::<Vec<_>>() {
            let (channel, message) = match channels.get(&container.id) {
                Some(channel) => *channel,
                None => {
                    let channel = http
                        .create_guild_channel(GUILD, &container.name)
                        .context("Failed to set channel name")?
                        .parent_id(CATEGORY)
                        .await
                        .context("Failed to create channel")?
                        .model()
                        .await
                        .context("Failed to get channel model")?;

                    info!(
                        "Created channel: {} for container: {}",
                        channel.id, container.id
                    );

                    channels.insert(container.id.clone(), (channel.id, None));

                    (channel.id, None)
                }
            };

            let logs = container
                .logs
                .iter()
                .map(|l| {
                    let message = match l {
                        LogOutput::StdErr { message } => message,
                        LogOutput::StdOut { message } => message,
                        LogOutput::StdIn { message } => message,
                        LogOutput::Console { message } => message,
                    }
                    .iter()
                    .copied()
                    .collect_vec();

                    String::from_utf8(message)
                        .unwrap_or_else(|e| format!("-- Failed to parse bytes as utf8: {}", e))
                        .chars()
                        .filter(|c| !c.is_ascii_control() && c.is_alphanumeric())
                        .collect::<String>()
                })
                .join("\n");

            let embeds = &[Embed {
                author: None,
                color: Some(0x4F759B),
                description: Some(format!(
                    "Container {} ({}) running {} is {} with image {}\n```{}\n```",
                    container.name,
                    container.id,
                    container.command,
                    container.status,
                    container.image,
                    if logs.len() > 1500 {
                        &logs[logs.len() - 1500..]
                    } else {
                        &logs
                    }
                )),
                fields: vec![],
                footer: Some(EmbedFooter {
                    icon_url: None,
                    proxy_icon_url: None,
                    text: format!("Updated at {}", chrono::Utc::now()),
                }),
                image: None,
                kind: "".to_string(),
                provider: None,
                thumbnail: None,
                timestamp: None,
                title: Some("Container Status".to_string()),
                url: None,
                video: None,
            }];

            let id = match message {
                Some(message) => {
                    http.update_message(channel, message)
                        .content(None)
                        .context("Failed to set message content")?
                        .embeds(Some(embeds))
                        .context("Failed to set message embeds")?
                        .await
                        .context("Failed to send message")?
                        .model()
                        .await
                        .context("Failed to get message model")?
                        .id
                }
                None => {
                    http.create_message(channel)
                        .content("")
                        .context("Failed to set message content")?
                        .embeds(embeds)
                        .context("Failed to set message embeds")?
                        .await
                        .context("Failed to send message")?
                        .model()
                        .await
                        .context("Failed to get message model")?
                        .id
                }
            };

            channels.insert(container.id.clone(), (channel, Some(id)));
        }

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
