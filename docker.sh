#!/bin/bash
TAG=master
full_tag="registry.egoroff.spb.ru/egoroff/bstore:${TAG}"
DOCKER_BUILDKIT=1 docker build . -t "${full_tag}-x64"
DOCKER_BUILDKIT=1 docker build . -f DockerfileArm64 -t "${full_tag}-arm64" --platform=linux/arm64
docker manifest create $full_tag --amend ${full_tag}-x64 --amend ${full_tag}-arm64
docker push $full_tag
