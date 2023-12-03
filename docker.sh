#!/bin/bash

tag="egoroff/bstore"
DOCKER_BUILDKIT=1 docker build . -t $tag
docker push $tag
