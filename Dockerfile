# Byg
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock* ./
COPY src src
COPY templates templates
COPY static static
COPY windows windows
RUN cargo build --release --locked

# Kør
FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/spilarr /usr/local/bin/spilarr
ENV GAMES_DIR=/games CONFIG_DIR=/config PORT=8790
EXPOSE 8790
USER 99:100
ENTRYPOINT ["/usr/local/bin/spilarr"]
