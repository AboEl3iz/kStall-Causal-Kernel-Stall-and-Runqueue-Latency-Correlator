# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall eBPF Makefile

CLANG ?= clang
BPFTOOL ?= bpftool
ARCH ?= $(shell uname -m | sed 's/x86_64/x86/' | sed 's/aarch64/arm64/')

BPF_CFLAGS := -g -O2 -target bpf \
	-D__TARGET_ARCH_$(ARCH) \
	-I. \
	-Wall \
	-Werror

all: kstall.bpf.o kstall.skel.h

vmlinux.h:
	$(BPFTOOL) btf dump file /sys/kernel/btf/vmlinux format c > $@

kstall.bpf.o: kstall.bpf.c kstall.h vmlinux.h
	$(CLANG) $(BPF_CFLAGS) -c $< -o $@

kstall.skel.h: kstall.bpf.o
	$(BPFTOOL) gen skeleton $< > $@

clean:
	rm -f kstall.bpf.o kstall.skel.h

.PHONY: all clean
