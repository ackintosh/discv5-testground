FROM rust:1.57-bullseye as builder
WORKDIR /usr/src/test-plan
COPY . .

# In `docker:generic` builder, the root of the docker build context is one directory higher than this test plan
# https://docs.testground.ai/builder-library/docker-generic#usage
RUN cd plan && cargo build

FROM debian:bullseye-slim
COPY --from=builder /usr/src/test-plan/plan/target/debug/test-plan-discv5 /usr/local/bin/test-plan-discv5
EXPOSE 6060
ENTRYPOINT [ "test-plan-discv5"]