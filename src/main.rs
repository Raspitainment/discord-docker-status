#![warn(clippy::all)]

use anyhow::Context;
use bollard::Docker;
use log::{error, info, warn};
use std::collections::HashMap;
use twilight_http::Client as HttpClient;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, MessageMarker},
    Id,
};

mod discord;
mod docker;

const GUILD: Id<GuildMarker> = Id::new(1209473653759016990);
const CATEGORY: Id<ChannelMarker> = Id::new(1218191011348615240);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    simplelog::TermLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )
    .unwrap();

    let token = std::env::var("DISCORD_TOKEN").context("Failed to get DISCORD_TOKEN")?;
    let discord = HttpClient::new(token);

    let docker = Docker::connect_with_socket_defaults().context("Failed to connect to docker")?;

    let mut message_cache = Cache::new();

    loop {
        run(&discord, &docker, &mut message_cache)
            .await
            .unwrap_or_else(|e| error!("Thread error: {:?}", e));

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

type Cache = HashMap<String, (Id<ChannelMarker>, Id<MessageMarker>)>;

async fn run(
    discord: &HttpClient,
    docker: &Docker,
    message_cache: &mut Cache,
) -> anyhow::Result<()> {
    let containers = docker::containers(docker)
        .await
        .context("Failed to get containers")?;

    info!(
        "Got containers {:?}",
        containers.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // remove channels for containers that no longer exist
    let mut to_remove = vec![];
    for (name, &(id, _)) in message_cache
        .iter()
        .filter(|&(container_name, _)| !containers.iter().any(|c| &c.name == container_name))
    {
        info!(
            "Removing channel (discord id: {}) for container: {}",
            name, id
        );

        discord
            .delete_channel(id)
            .await
            .context("Failed to delete channel")?;

        to_remove.push(name.clone());
    }
    to_remove.into_iter().for_each(|name| {
        message_cache.remove(&name);
    });

    // create channels for containers that don't have one
    let mut to_add = vec![];
    for container in containers
        .iter()
        .filter(|c| !message_cache.contains_key(&c.name))
    {
        info!("Creating channel for container: {}", container.name);

        let channel = discord
            .create_guild_channel(GUILD, &container.name)
            .context("Failed to set up channel")?
            .parent_id(CATEGORY)
            .await
            .context("Failed to create channel")?
            .model()
            .await
            .context("Failed to get channel model")?;

        let message = discord
            .create_message(channel.id)
            .content("> Content goes here...")
            .context("Failed to set up message")?
            .await
            .context("Failed to create message")?
            .model()
            .await
            .context("Failed to get message model")?;

        to_add.push((container.name.clone(), (channel.id, message.id)));
    }
    to_add.into_iter().for_each(|(name, (channel, message))| {
        message_cache.insert(name, (channel, message));
    });

    // update messages in channels
    for (container_name, (channel_id, message_id)) in message_cache.iter() {
        info!("Updating message for container: {}", container_name);

        let container = containers
            .iter()
            .find(|c| &c.name == container_name)
            .context("Failed to find container")?;

        let logs = docker::logs(docker, &container.id).await;

        let embed = discord::embed_container(container, &logs).context("Failed to create embed")?;

        discord
            .update_message(*channel_id, *message_id)
            .content(None)
            .context("Failed to set up message update")?
            .embeds(Some(&[embed]))
            .context("Failed to set up message update")?
            .await
            .context("Failed to update message")?
            .model()
            .await
            .context("Failed to get message model")?;
    }

    anyhow::Ok(())
}
