FROM rust:1.38-stretch

ENV CONTAINER_IMAGE_VER=v1.0.0

RUN echo $CONTAINER_IMAGE_VER

RUN rustup update 2>&1
RUN rustup component add rls rust-analysis rust-src 2>&1

RUN cargo install cargo-watch

# Configure apt
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get -y install --no-install-recommends apt-utils 2>&1

# Install other dependencies
RUN apt-get install -y lldb-3.9

# Install git, process tools, lsb-release (common in install instructions for CLIs)
RUN apt-get -y install sudo \
    curl \
    git-core \
    gnupg \
    linuxbrew-wrapper \
    locales \
    lsb-release \
    nano \
    procps \
    wget \
    # set up locale
    && locale-gen en_US.UTF-8

# Clean up
RUN apt-get autoremove -y \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/*

ENV DEBIAN_FRONTEND=dialog

# Set the default shell to bash rather than sh
ENV SHELL /bin/bash