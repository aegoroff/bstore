#!/bin/bash

tag="ghcr.io/aegoroff/bstore:master"
DOCKER_BUILDKIT=1 docker build . -t $tag
docker push $tag
