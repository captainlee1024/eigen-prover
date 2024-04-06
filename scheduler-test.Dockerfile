# Stage 1: Build
FROM rust:latest AS builder

RUN apt-get update && apt-get install -y \
    autoconf \
    automake \
    libtool \
    curl \
    make \
    gcc \
    g++ \
    unzip \
    libpq-dev \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/* \

WORKDIR /app

COPY . /app/

RUN cd app/service && cargo test --no-run --release

# Stage 2: Run
FROM debian:stable-slim

RUN apt-get update && apt-get install -y --no-install-recommends apt
RUN apt-get update && apt-get install -y --no-install-recommends openssl
RUN apt-get update && apt-get install -y --no-install-recommends libpq5

COPY --from=builder /app/service/target/release/deps /usr/local/bin/tests

RUN adduser --disabled-password --gecos '' --uid 1000 appuser && chown -R appuser:appuser /usr/local/bin/tests
USER appuser

ENV TEST_NAME=your_test_name

CMD ["/usr/local/bin/tests/$TEST_NAME"]