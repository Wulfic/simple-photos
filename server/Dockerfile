FROM rust:1-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm src/main.rs
COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
# Run full package upgrade first to pick up all bookworm-security backports,
# eliminating OS-level CVEs before adding application packages.
RUN apt-get update && apt-get upgrade -y --no-install-recommends && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libsqlite3-0 \
    curl \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd -r appuser && useradd -r -g appuser -d /app -s /sbin/nologin appuser
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/simple-photos-server ./
COPY --from=builder /usr/src/app/migrations ./migrations
RUN mkdir -p /data/storage /data/db && chown -R appuser:appuser /data /app
ENV SIMPLE_PHOTOS_CONFIG=/app/config.toml
EXPOSE 3000
USER appuser
ENTRYPOINT ["./simple-photos-server"]
