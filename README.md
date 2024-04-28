# Discord Duild Integration

Get the current status of your Docker Build via Webhook in Discord.

## Example

```
docker run -d --name container -v /var/run/docker.sock:/var/run/docker.sock -e DISCORD_TOKEN=xyz -it image
```
