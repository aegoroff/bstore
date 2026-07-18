#!/usr/bin/env bash
set -euo pipefail

TAG="${TAG:-master}"
full_tag="registry.egoroff.spb.ru/egoroff/bstore:${TAG}"
builder="bstore-multiarch"

if ! docker buildx inspect "${builder}" >/dev/null 2>&1; then
  docker buildx create --name "${builder}" --driver docker-container --use
else
  docker buildx use "${builder}"
fi

# Per-arch images. --provenance=false keeps a plain image manifest (no attestation index).
docker buildx build \
  --platform linux/amd64 \
  -t "${full_tag}-x64" \
  --provenance=false \
  --push \
  .

docker buildx build \
  --platform linux/arm64 \
  -t "${full_tag}-arm64" \
  --provenance=false \
  --push \
  .

# registry:3 + buildx imagetools often fails with HEAD 400 on an existing tag.
# docker manifest list (Schema 2) is what this registry already accepts.
docker manifest rm "${full_tag}" 2>/dev/null || true
docker manifest create "${full_tag}" "${full_tag}-x64" "${full_tag}-arm64"
docker manifest push --purge "${full_tag}"

echo "Pushed multi-arch manifest: ${full_tag}"
docker buildx imagetools inspect "${full_tag}"
