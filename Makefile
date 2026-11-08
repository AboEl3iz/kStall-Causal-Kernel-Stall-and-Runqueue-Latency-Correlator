# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall Root Makefile

CARGO ?= cargo
ARCH ?= $(shell uname -m | sed 's/x86_64/x86/' | sed 's/aarch64/arm64/')

.PHONY: all bpf build release test check lint clippy fmt fmt-check ci package verify-release docker doctor install clean run-watch run-top

all: bpf build

bpf:
	$(MAKE) -C bpf all

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --workspace --release

test:
	$(CARGO) test --workspace

check:
	$(CARGO) check --workspace

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

lint:
	./ci/lint.sh

ci:
	./ci/test.sh

package:
	./ci/release.sh

verify-release:
	./ci/verify-release.sh

docker:
	./ci/docker.sh

doctor: build
	./target/debug/kstall doctor

install:
	./install.sh

run-watch: build
	sudo ./target/debug/kstall watch --threshold 2ms

run-top: build
	sudo ./target/debug/kstall top --threshold 2ms

clean:
	$(MAKE) -C bpf clean
	$(CARGO) clean
	rm -rf dist
