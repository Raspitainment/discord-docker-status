#![warn(clippy::all)]

use anyhow::Context;
use bollard::{
    container::{ListContainersOptions, LogOutput, LogsOptions},
    Docker,
};
use futures::StreamExt;
use itertools::Itertools;
use log::{error, info, warn};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use twilight_http::Client as HttpClient;
use twilight_model::{
    channel::message::{
        embed::{EmbedAuthor, EmbedFooter},
        Embed,
    },
    id::{
        marker::{ChannelMarker, GuildMarker},
        Id,
    },
    util::Timestamp,
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

    let containers = Arc::new(Mutex::new(Store {
        containers: HashMap::new(),
        to_remove: Vec::new(),
    }));

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

async fn container_thread(store: Arc<Mutex<Store>>) -> anyhow::Result<()> {
    let docker = Docker::connect_with_socket_defaults().context("Failed to connect to Docker")?;

    loop {
        info!("Updating containers");

        let mut new_store = HashMap::new();

        let containers = docker
            .list_containers::<&str>(Some(ListContainersOptions {
                all: true,
                limit: None,
                size: false,
                filters: Default::default(),
            }))
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
                        tail: "40",
                    }),
                )
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

            info!("Got {} logs", logs.len());

            new_store
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

        {
            let mut store = store.lock().await;
            store.to_remove = store
                .containers
                .keys()
                .filter(|&name| !new_store.contains_key(name))
                .cloned()
                .collect();
            store.containers = new_store;
        }

        info!("Updated containers");

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

struct Store {
    containers: HashMap<String, Container>,
    to_remove: Vec<String>,
}

async fn message_update(store: Arc<Mutex<Store>>) -> anyhow::Result<()> {
    let token = std::env::var("DISCORD_TOKEN").context("Failed to get DISCORD_TOKEN")?;

    let http = Arc::new(HttpClient::new(token));

    let mut channels = HashMap::<String, _>::new();

    tokio::time::sleep(std::time::Duration::from_secs(15)).await;

    loop {
        info!("Updating messages");

        for name in store.lock().await.to_remove.drain(..) {
            for channel in http
                .guild_channels(GUILD)
                .await
                .context("Failed to get guild channels")?
                .model()
                .await
                .context("Failed to get guild channels model")?
                .iter()
                .filter(|c| c.parent_id == Some(CATEGORY))
                .filter(|c| c.name.as_ref() == Some(&name))
            {
                http.delete_channel(channel.id)
                    .await
                    .context("Failed to delete channel")?;

                channels.remove(&name);

                info!("Deleted channel: {:?} ({})", channel.name, channel.id);
            }
        }

        for container in store.lock().await.containers.values().collect::<Vec<_>>() {
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

                    strip_ansi_escapes::strip_str(
                        String::from_utf8(message)
                            .unwrap_or_else(|e| format!("-- Failed to parse bytes as utf8: {}", e)),
                    )
                    .chars()
                    .collect::<String>()
                })
                .collect::<String>();

            let embed = &[Embed {
                author: Some(EmbedAuthor {
                    icon_url: None,
                    name: format!("{} ({})", container.name, container.id),
                    proxy_icon_url: None,
                    url: None,
                }),
                color: Some(0x3772FF),
                description: Some(format!(
                    "Image `{}`\nRunning `{}`:\n```{}```",
                    container.image,
                    container.command,
                    &logs[(logs.len() as i64 - 3900).max(0) as usize..],
                )),
                fields: vec![],
                footer: Some(EmbedFooter {
                    icon_url: None,
                    proxy_icon_url: None,
                    text: format!("{} ({})", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
                }),
                image: None,
                kind: "rich".to_string(),
                provider: None,
                thumbnail: None,
                timestamp: Some(
                    Timestamp::from_micros(chrono::Utc::now().timestamp_micros())
                        .context("Failed to get timestamp")?,
                ),
                title: Some(container.status.clone()),
                url: None,
                video: None,
            }];

            let id = match message {
                Some(message) => {
                    http.update_message(channel, message)
                        .content(Some(""))
                        .context("Failed to set message content")?
                        .embeds(Some(embed))
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
                        .embeds(embed)
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

        info!("Updated messages");

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
