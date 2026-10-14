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
RUN cargo build --release --locked -p anthill-cli -p anthill-dapr

# ---- runtime ----
FROM debian:bookworm-slim
RUN useradd --system --create-home --uid 10001 --shell /usr/sbin/nologin anthill
COPY --from=build /src/target/release/anthill /usr/local/bin/anthill
COPY --from=build /src/target/release/anthill-dapr-sim /usr/local/bin/anthill-dapr-sim
USER anthill
WORKDIR /home/anthill
EXPOSE 7878
ENTRYPOINT ["anthill"]
CMD ["serve", "--addr", "0.0.0.0:7878"]
