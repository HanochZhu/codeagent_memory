FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
LABEL io.modelcontextprotocol.server.name="io.github.hanochzhu/codeagent-memory"
LABEL org.opencontainers.image.source="https://github.com/HanochZhu/codeagent_memory"
LABEL org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/cam /usr/local/bin/cam
WORKDIR /work
ENTRYPOINT ["cam"]
CMD ["mcp"]
