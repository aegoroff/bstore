#!/bin/bash

tag="ghcr.io/aegoroff/bstore:master"
DOCKER_BUILDKIT=1 docker build . -t "${tag}-x64"
docker push "${tag}-x64"
DOCKER_BUILDKIT=1 docker build . -f DockerfileArm64 -t "${tag}-arm64" --platform=linux/arm64
docker push "${tag}-arm64"
