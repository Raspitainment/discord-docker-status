use anyhow::Context;
use bollard::container::LogOutput;
use itertools::Itertools;
use twilight_model::{
    channel::message::{
        embed::{EmbedAuthor, EmbedFooter},
        Embed,
    },
    util::Timestamp,
};

use crate::docker::Container;

pub fn embed_container(container: &Container, logs: &[LogOutput]) -> anyhow::Result<Embed> {
    let logs = logs
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

    let embed = Embed {
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
    };

    Ok(embed)
}
