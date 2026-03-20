FROM rust:1.88-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm src/main.rs
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libsqlite3-0 curl ffmpeg && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/simple-photos-server ./
COPY --from=builder /usr/src/app/migrations ./migrations
RUN mkdir -p /data/storage /data/db
ENV SIMPLE_PHOTOS_CONFIG=/app/config.toml
EXPOSE 3000
ENTRYPOINT ["./simple-photos-server"]
