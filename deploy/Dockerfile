# Multi-stage Dockerfile for kStall
FROM rust:bookworm AS builder

# Install LLVM, Clang, bpftool, and libbpf
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    llvm \
    libbpf-dev \
    linux-libc-dev \
    libelf-dev \
    zlib1g-dev \
    make \
    pkg-config \
    bpftool \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt

WORKDIR /src
COPY . .

# Build BPF programs and userspace binary
RUN make -C bpf all
RUN cargo build --release

# Final runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libelf1 \
    libbpf1 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/kstall /usr/local/bin/kstall

ENTRYPOINT ["/usr/local/bin/kstall"]
CMD ["watch", "--format", "json"]
