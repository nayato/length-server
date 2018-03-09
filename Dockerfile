### Builder Image

FROM rust:latest as builder
WORKDIR /app/src
RUN USER=root cargo new --bin length-server && echo fail_it_intentionally >> ./length-server/src/main.rs
COPY Cargo.toml Cargo.lock ./length-server/

WORKDIR /app/src/length-server
#RUN RUSTFLAGS="-C target-cpu=native" cargo build --release; exit 0
RUN cargo build --release; exit 0
RUN rm src/main.rs

COPY ./src/ /app/src/length-server/src/
# RUN RUSTFLAGS="-C target-cpu=native" cargo build --release
RUN cargo build --release


### Final Image

FROM debian:stretch-slim
WORKDIR /app
RUN apt update \
    && apt install -y openssl ca-certificates \
    && apt clean \
    && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*

EXPOSE 21000 21001 21002 21080 21081 21082

COPY --from=builder /app/src/length-server/target/release/length-server ./
COPY gateway.tests.com.pfx ./

CMD ["/app/length-server"]
