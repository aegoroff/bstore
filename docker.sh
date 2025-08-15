#!/bin/bash

tag="ghcr.io/aegoroff/bstore:master"
DOCKER_BUILDKIT=1 docker build . -t $tag
DOCKER_BUILDKIT=1 docker build . -f DockerfileArm64 -t $tag --platform=linux/arm64
docker push $tag
