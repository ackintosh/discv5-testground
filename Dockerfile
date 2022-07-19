FROM rust:1.62-bullseye as builder
WORKDIR /usr/src/test-plan

# Cache dependencies between test runs,
# See https://blog.mgattozzi.dev/caching-rust-docker-builds/
# And https://github.com/rust-lang/cargo/issues/2644
RUN mkdir -p ./plan/src/
RUN echo "fn main() { println!(\"If you see this message, you may want to clean up the target directory or the Docker build cache.\") }" > ./plan/src/main.rs
COPY ./plan/Cargo.lock ./plan/
COPY ./plan/Cargo.toml ./plan/
RUN cd ./plan/ && cargo build --release

COPY . .

# This is in order to make sure `main.rs`s mtime timestamp is updated to avoid the dummy `main`
# remaining in the release binary.
# https://github.com/rust-lang/cargo/issues/9598
RUN touch ./plan/src/main.rs

# In `docker:generic` builder, the root of the docker build context is one directory higher than this test plan
# https://docs.testground.ai/builder-library/docker-generic#usage
RUN cd plan && cargo install --locked --path .

FROM debian:bullseye-slim
COPY --from=builder /usr/local/cargo/bin/discv5-testground /usr/local/bin/discv5-testground

ENV RUST_LOG=discv5=debug

ENTRYPOINT ["discv5-testground"]