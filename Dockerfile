# syntax=docker/dockerfile:1

# ---- web UI ----
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# ---- binaries ----
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
# The UI is embedded into the server binary at compile time.
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --locked -p hairspring-cli -p hairspring-dapr

# ---- runtime ----
FROM debian:bookworm-slim
RUN useradd --system --create-home --uid 10001 --shell /usr/sbin/nologin hairspring
COPY --from=build /src/target/release/hairspring /usr/local/bin/hairspring
COPY --from=build /src/target/release/hairspring-dapr-sim /usr/local/bin/hairspring-dapr-sim
USER hairspring
WORKDIR /home/hairspring
EXPOSE 7878
ENTRYPOINT ["hairspring"]
CMD ["serve", "--addr", "0.0.0.0:7878"]
