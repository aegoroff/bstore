#!/bin/bash

tag="egoroff/bstore"
docker build . -t $tag
docker push $tag
