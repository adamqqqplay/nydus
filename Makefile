# Variables
TEST_WORKDIR_PREFIX ?= "/tmp"

current_dir := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))

env_go_path := $(shell go env GOPATH 2> /dev/null)
go_path := $(if $(env_go_path),$(env_go_path),"$(HOME)/go")

# Set the env DIND_CACHE_DIR to specify a cache directory for
# docker-in-docker container, used to cache data for docker pull,
# then mitigate the impact of docker hub rate limit, for example:
# env DIND_CACHE_DIR=/path/to/host/var-lib-docker make docker-nydusify-smoke
dind_cache_mount := $(if $(DIND_CACHE_DIR),-v $(DIND_CACHE_DIR):/var/lib/docker,)

# Functions

# Func: build golang target in docker
# Args:
#   $(1): target make command
define build_golang
	@echo "Building golang: $(1)"
	docker run --rm -v ${go_path}:/go -v ${current_dir}:/nydus-rs --workdir /nydus-rs golang:1.14 $(1)
endef

# Targets

build: build-virtiofs build-fusedev
	cargo fmt -- --check

release: build-virtiofs-release build-fusedev-release
	cargo fmt -- --check

build-virtiofs:
	# TODO: switch to --out-dir when it moves to stable
	# For now we build with separate target directories
	cargo build --features=virtiofs --target-dir target-virtiofs
	cargo clippy --features=virtiofs --tests --bins --workspace --target-dir target-virtiofs  -- -Dclippy::all

build-fusedev:
	cargo build --features=fusedev --target-dir target-fusedev
	cargo clippy --features=fusedev --tests --bins --workspace --target-dir target-fusedev  -- -Dclippy::all

build-virtiofs-release:
	cargo build --features=virtiofs --release --target-dir target-virtiofs

build-fusedev-release:
	cargo build --features=fusedev --release --target-dir target-fusedev

static-release:
	cargo build --target x86_64-unknown-linux-musl --features=fusedev --release --target-dir target-fusedev
	cargo build --target x86_64-unknown-linux-musl --features=virtiofs --release --target-dir target-virtiofs

ut:
	RUST_BACKTRACE=1 cargo test --features=fusedev --target-dir target-fusedev --workspace -- --nocapture --test-threads=15 --skip integration
	RUST_BACKTRACE=1 cargo test --features=virtiofs --target-dir target-virtiofs --workspace -- --nocapture --test-threads=15 --skip integration

docker-static:
	docker build -t nydus-rs-static misc/musl-static
	docker run --rm \
		-v ${current_dir}:/nydus-rs \
		--workdir /nydus-rs \
		-v ~/.ssh/id_rsa:/root/.ssh/id_rsa \
		-v ~/.cargo/git:/root/.cargo/git \
		-v ~/.cargo/registry:/root/.cargo/registry \
		nydus-rs-static

# Run smoke test including general integration tests and unit tests in container.
# Nydus binaries should already be prepared.
static-test:
	# No clippy for virtiofs for now since it has much less updates.
	cargo clippy --features=fusedev --tests --bins --workspace --target-dir target-fusedev  -- -Dclippy::all
	# For virtiofs target UT
	cargo test --target x86_64-unknown-linux-musl --features=virtiofs --release --target-dir target-virtiofs --workspace -- --nocapture --test-threads=15 --skip integration
	# For fusedev target UT & integration
	cargo test --target x86_64-unknown-linux-musl --features=fusedev --release --target-dir target-fusedev --workspace -- --nocapture --test-threads=15

docker-nydus-smoke: docker-static
	docker build -t nydus-smoke misc/nydus-smoke
	docker run --rm --privileged \
		-e TEST_WORKDIR_PREFIX=$(TEST_WORKDIR_PREFIX) \
		-v $(TEST_WORKDIR_PREFIX) \
		-v ${current_dir}:/nydus-rs \
		-v ~/.ssh/id_rsa:/root/.ssh/id_rsa \
		-v ~/.cargo/git:/root/.cargo/git \
		-v ~/.cargo/registry:/root/.cargo/registry \
		nydus-smoke

docker-nydusify-smoke: docker-static
	$(call build_golang,make -C contrib/nydusify build-smoke)
	docker build -t nydusify-smoke misc/nydusify-smoke
	docker run --rm --privileged \
		-e BACKEND_TYPE=$(BACKEND_TYPE) \
		-e BACKEND_CONFIG=$(BACKEND_CONFIG) \
		-v $(current_dir):/nydus-rs $(dind_cache_mount) nydusify-smoke TestSmoke

docker-nydusify-image-test: docker-static
	$(call build_golang,make -C contrib/nydusify build-smoke)
	docker build -t nydusify-smoke misc/nydusify-smoke
	docker run --rm --privileged \
		-e BACKEND_TYPE=$(BACKEND_TYPE) \
		-e BACKEND_CONFIG=$(BACKEND_CONFIG) \
		-v $(current_dir):/nydus-rs $(dind_cache_mount) nydusify-smoke TestDockerHubImage

docker-smoke: docker-nydus-smoke docker-nydusify-smoke

nydusify:
	$(call build_golang,make -C contrib/nydusify)
nydusify-static:
	$(call build_golang,make -C contrib/nydusify static-release)

nydus-snapshotter:
	$(call build_golang,make -C contrib/nydus-snapshotter)
nydus-snapshotter-static:
	$(call build_golang,make -C contrib/nydus-snapshotter static-release)

# Run integration smoke test in docker-in-docker container. It requires some special settings,
# refer to `misc/example/README.md` for details.
all-static-release: docker-static nydusify-static nydus-snapshotter-static

# https://www.gnu.org/software/make/manual/html_node/One-Shell.html
.ONESHELL:
docker-example: all-static-release
	cp ${current_dir}/target-fusedev/x86_64-unknown-linux-musl/release/nydusd misc/example
	cp ${current_dir}/target-fusedev/x86_64-unknown-linux-musl/release/nydus-image misc/example
	cp contrib/nydusify/cmd/nydusify misc/example
	cp contrib/nydus-snapshotter/bin/containerd-nydus-grpc misc/example
	docker build -t nydus-rs-example misc/example
	@cid=$(shell docker run --rm -t -d --privileged $(dind_cache_mount) nydus-rs-example)
	@docker exec $$cid /run.sh
	@EXIT_CODE=$$?
	@docker rm -f $$cid
	@exit $$EXIT_CODE
