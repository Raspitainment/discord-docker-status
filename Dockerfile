FROM debian:bookworm

ENV DEBIAN_FRONTEND=noninteractive

RUN apt update --yes
RUN apt upgrade --yes
RUN apt install --yes curl build-essential

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

COPY . /app
WORKDIR /app
ENV DISCORD_TOKEN=your_token_here

CMD ["/root/.cargo/bin/cargo", "run"]
