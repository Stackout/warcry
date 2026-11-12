FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p warcry-hub

FROM debian:stable-slim
RUN useradd --system --no-create-home warcry
COPY --from=build /src/target/release/warcry-hub /usr/local/bin/warcry-hub
USER warcry
EXPOSE 8080
ENTRYPOINT ["warcry-hub"]
