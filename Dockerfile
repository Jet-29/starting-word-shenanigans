FROM rust:latest AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && touch src/main.rs
RUN cargo fetch
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
#FROM alpine
WORKDIR /usr/local/bin/

COPY valid-words.txt /usr/local/bin/
COPY --from=build /app/target/release/starting-word-shenanigans /usr/local/bin/
CMD ["/usr/local/bin/starting-word-shenanigans"]

