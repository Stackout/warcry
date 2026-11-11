#!/usr/bin/env bash
set -u
packages=(
  libasound2-dev libx11-dev libxi-dev libxtst-dev cmake clang libclang-dev
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev file
)
mirrors=(
  http://azure.archive.ubuntu.com/ubuntu/
  http://archive.ubuntu.com/ubuntu/
  http://mirrors.edge.kernel.org/ubuntu/
  http://mirrors.mit.edu/ubuntu/
  http://mirror.math.princeton.edu/pub/ubuntu/
)
host() { echo "$1" | sed -E 's#^https?://([^/]+)/.*#\1#'; }
resolvable() { getent hosts "$(host "$1")" > /dev/null; }

resolvable "${mirrors[0]}" || resolvable "${mirrors[1]}" || {
  echo "::warning::runner DNS can't resolve Ubuntu mirrors, switching to public DNS"
  printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' | sudo tee /etc/resolv.conf > /dev/null
}

usable=()
for m in "${mirrors[@]}"; do resolvable "$m" && usable+=("$m"); done
echo "usable mirrors: ${usable[*]:-none}"
test ${#usable[@]} -gt 0 && {
  printf '%s\n' "${usable[@]}" | sudo tee /etc/apt/apt-mirrors.txt > /dev/null
  sudo sed -i "s#http://azure.archive.ubuntu.com/ubuntu/\?#${usable[0]}#g" /etc/apt/sources.list /etc/apt/sources.list.d/* 2>/dev/null
}

apt=(sudo timeout 600 apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
for attempt in 1 2 3; do
  "${apt[@]}" update && "${apt[@]}" install -y --no-install-recommends "${packages[@]}" && exit 0
  echo "::warning::apt attempt $attempt failed"
  sleep 15
done
exit 1
