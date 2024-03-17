use anyhow::Context;
use bollard::{
    container::{ListContainersOptions, LogOutput, LogsOptions},
    Docker,
};
use futures::StreamExt;
use itertools::Itertools;
use log::{info, warn};

pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub command: String,
    pub status: String,
}

pub async fn logs(docker: &Docker, id: &str) -> Vec<LogOutput> {
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

    info!("Got {} logs for container: {}", logs.len(), id);

    logs
}

pub async fn containers(docker: &Docker) -> anyhow::Result<Vec<Container>> {
    let cs = docker
        .list_containers::<&str>(Some(ListContainersOptions {
            all: true,
            limit: None,
            size: false,
            filters: Default::default(),
        }))
        .await
        .context("Failed to list containers")?;

    let containers = cs
        .into_iter()
        .filter_map(|c| {
            let x = [
                c.id.clone(),
                c.names.as_ref().and_then(|names| names.first().cloned()),
                c.image.clone(),
                c.command.clone(),
                c.status.clone(),
            ]
            .into_iter()
            .collect::<Option<Vec<_>>>();

            let Some([id, name, image, command, status]) = x.as_deref() else {
                warn!("Container has missing fields: {:?}", c);
                return None;
            };

            Some(Container {
                id: id.to_string(),
                name: name.to_string(),
                image: image.to_string(),
                command: command.to_string(),
                status: status.to_string(),
            })
        })
        .collect_vec();

    Ok(containers)
}
