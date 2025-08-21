#!/bin/bash

tag="registry.egoroff.spb.ru/egoroff/bstore:master"
DOCKER_BUILDKIT=1 docker build . -t "${tag}-x64"
DOCKER_BUILDKIT=1 docker build . -f DockerfileArm64 -t "${tag}-arm64" --platform=linux/arm64
docker manifest create $tag --amend ${tag}-x64 --amend ${tag}-arm64
docker push $tag
