FROM rust:1.88-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config=1.8.1-1 libssl-dev=3.0.19-1~deb12u2 && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm src/main.rs
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates=20230311+deb12u1 libsqlite3-0=3.40.1-2+deb12u2 curl=7.88.1-10+deb12u14 ffmpeg=7:5.1.8-0+deb12u1 && rm -rf /var/lib/apt/lists/*
RUN groupadd -r appuser && useradd -r -g appuser -d /app -s /sbin/nologin appuser
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/simple-photos-server ./
COPY --from=builder /usr/src/app/migrations ./migrations
RUN mkdir -p /data/storage /data/db && chown -R appuser:appuser /data /app
ENV SIMPLE_PHOTOS_CONFIG=/app/config.toml
EXPOSE 3000
USER appuser
ENTRYPOINT ["./simple-photos-server"]
