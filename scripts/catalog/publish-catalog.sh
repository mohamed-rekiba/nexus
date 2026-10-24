#!/usr/bin/env bash
# Publishes the catalogs to the `catalog` branch of a git remote, as Nexus fetches them: the
# files `node publish.mjs` writes (ps5.json, ps4.json, pc.json) at the top,
# for the "Catalog address" setting to name the folder they are in. sources/ keeps the Russian
# listings and the translation cache the next refresh starts from. The branch keeps one commit,
# replaced each time.
#
#   scripts/catalog/publish-catalog.sh [dist/catalog] [git remote URL]
#
# Nexus fetches a catalog only from a host in the release-catalog plugin's manifest
# (plugins/services/release-catalog/plugin.toml, `network`). Add the host that serves this
# branch there first.
set -euo pipefail
src=$(realpath "${1:-dist/catalog}")
# The catalog names and forum keys, from the project's assets/catalog-sources.json.
sources="$(dirname "$0")/../../assets/catalog-sources.json"
names=$(jq -r 'keys_unsorted[]' "$sources")
forums=$(jq -r '.[].forums[].key' "$sources")
remote=${2:-$(git remote get-url origin)}
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
found=0
for name in $names; do
  if [ -f "$src/publish/$name.json" ]; then
    cp "$src/publish/$name.json" "$out/$name.json"
    found=1
  fi
done
if [ "$found" = 0 ]; then
  echo "no catalog in $src/publish: run node publish.mjs first" >&2
  exit 1
fi
mkdir "$out/sources"
for p in $forums; do
  if [ -f "$src/$p-topics.json.ru-backup.json" ]; then
    gzip -9nc "$src/$p-topics.json.ru-backup.json" >"$out/sources/$p-topics.ru.json.gz"
  fi
done
if [ -f "$src/english-translations.json" ]; then
  gzip -9nc "$src/english-translations.json" >"$out/sources/english-translations.json.gz"
fi
cat >"$out/README.md" <<'MD'
# Game catalog

The catalog files Nexus reads, one for each console. Set Nexus's "Catalog address" to the
folder that holds them. Don't commit here: each refresh replaces this branch.
MD
cd "$out"
git init -q -b catalog
git add .
git -c user.name="catalog bot" -c user.email="actions@users.noreply.github.com" commit -qm "Game catalog $(date -u +%F)"
git push -qf "$remote" catalog
for name in $names; do
  if [ -f "$name.json" ]; then echo "Published $name.json: $(jq -r .topic_count "$name.json") topics"; fi
done
