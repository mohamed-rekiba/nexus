#!/usr/bin/env bash
# Makes or updates the catalogs Nexus bundles: assets/catalog/ps5.json, ps4.json and pc.json.
#
#   scripts/catalog/update-catalogs.sh [--console ps5|ps4|pc] [--refresh] [--dry-run]
#
# --console reads only that catalog's forums, and replaces only that catalog. Without it, every
# catalog in assets/catalog-sources.json is made.
#
# Without --refresh (the full run), each forum in assets/catalog-sources.json is read again in a
# visible Chrome window: do the Cloudflare check yourself when it shows. Its new listing becomes
# the Russian source, and translate.mjs makes it English. Set STEAM=1 to add Steam art and
# ProtonDB ratings (steam.mjs) before the translation.
#
# With --refresh, no browser opens: refresh.mjs updates each Russian source from the forum's
# feed and API. A forum needs a Russian source from an earlier full run; one without it is
# skipped.
#
# Then publish.mjs writes the catalogs, and they are copied into dist/catalog/publish/ and
# assets/catalog/. Before anything changes, the current assets/catalog/*.json and Russian sources
# are copied to dist/catalog/backup/<UTC time>/. The run stops at the first forum whose listing
# is partial, and never copies a file that is not complete and English.
#
# --dry-run prints each command and copy without doing it.
#
# The files in assets/catalog/ are your data: don't commit them.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
nexus=$(cd "$here/../.." && pwd)
sources="$nexus/assets/catalog-sources.json"
dist="$nexus/dist/catalog"
assets="$nexus/assets/catalog"

refresh=0
dry=0
console=""
while [ $# -gt 0 ]; do
  case $1 in
    --refresh) refresh=1 ;;
    --dry-run) dry=1 ;;
    --console)
      [ $# -ge 2 ] || { echo "--console needs a catalog name, for example pc" >&2; exit 64; }
      console=$2
      shift
      ;;
    -h | --help)
      sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $1 (use --console, --refresh, --dry-run or --help)" >&2
      exit 64
      ;;
  esac
  shift
done

# Prints a command, then runs it unless this is a dry run.
run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [ "$dry" = 0 ]; then "$@"; fi
}

fail() {
  echo "update-catalogs: $*" >&2
  exit 1
}

# Checks.
command -v jq >/dev/null || fail "jq is required: install it"
command -v node >/dev/null || fail "Node 20 or later is required: install it"
node_major=$(node -p 'process.versions.node.split(".")[0]')
[ "$node_major" -ge 20 ] || fail "Node 20 or later is required: this is Node $(node -v)"
if [ ! -d "$here/node_modules" ]; then
  command -v npm >/dev/null || fail "npm is required to install the collector's packages"
  run npm --prefix "$here" install
fi
if [ "$refresh" = 0 ]; then
  node --input-type=module -e "import { findChrome } from '$here/chrome.mjs'; process.exit(findChrome() ? 0 : 1);" \
    || fail "Google Chrome or Chromium is required: install it, or set CHROME_PATH to its program"
fi

if [ -n "$console" ]; then
  jq -e --arg c "$console" 'has($c)' "$sources" >/dev/null \
    || fail "no catalog named $console in $sources: use one of $(jq -r 'keys_unsorted | join(", ")' "$sources")"
  names=$console
  forums=$(jq -r --arg c "$console" '.[$c].forums[].key' "$sources")
else
  names=$(jq -r 'keys_unsorted[]' "$sources")
  forums=$(jq -r '.[].forums[].key' "$sources")
fi
[ -n "$forums" ] || fail "no forums in $sources"

# Back up the bundled catalogs and the Russian sources before anything changes them.
backup="$dist/backup/$(date -u +%Y%m%dT%H%M%SZ)"
echo "Backup of the current catalogs and Russian sources: $backup"
run mkdir -p "$backup"
for name in $names; do
  if [ -f "$assets/$name.json" ]; then run cp -p "$assets/$name.json" "$backup/$name.json"; fi
done
for key in $forums; do
  source="$dist/$key-topics.json.ru-backup.json"
  if [ -f "$source" ]; then run cp -p "$source" "$backup/"; fi
done

if [ "$refresh" = 0 ]; then
  for key in $forums; do
    echo "== $key: read the forum (a Chrome window opens; do the Cloudflare check when it shows)"
    status=0
    run node "$here/list.mjs" --platform "$key" --include-magnets || status=$?
    if [ "$status" = 2 ]; then
      fail "the $key listing is partial (verification, login or an HTTP failure): see $dist/$key-topics.json.partial.json. Nothing was copied into $assets. Run again to read it once more."
    elif [ "$status" != 0 ]; then
      fail "list.mjs failed for $key (exit $status). Nothing was copied into $assets."
    fi
    # The new listing becomes the Russian source. translate.mjs and steam.mjs then work from it,
    # not from the source of an earlier run.
    run cp "$dist/$key-topics.json" "$dist/$key-topics.json.ru-backup.json"
    if [ "${STEAM:-0}" = 1 ]; then
      run node "$here/steam.mjs" --platform "$key"
    fi
    run node "$here/translate.mjs" --platform "$key" --from-original
  done
else
  for key in $forums; do
    if [ ! -f "$dist/$key-topics.json.ru-backup.json" ]; then
      echo "== $key: skipped, no Russian source $dist/$key-topics.json.ru-backup.json. Run make catalog first."
      continue
    fi
    echo "== $key: refresh from the feed and the forum list"
    run node "$here/refresh.mjs" --platform "$key"
    run node "$here/translate.mjs" --platform "$key" --from-original
  done
fi

# Publish into a new folder, so a file from an earlier run is never copied.
if [ "$dry" = 0 ]; then
  out=$(mktemp -d "$dist/publish-run.XXXXXX")
  trap 'rm -rf "$out"' EXIT
else
  out="$dist/publish-run.XXXXXX"
fi
run node "$here/publish.mjs" --output "$out"

# Check every file before the first copy, so a bad file leaves assets/catalog as it was.
if [ "$dry" = 0 ]; then
  for name in $names; do
    file="$out/$name.json"
    if [ -f "$file" ] && ! jq -e '.complete == true and .translation.language == "en"' "$file" >/dev/null; then
      fail "$file is not a complete, English catalog. Nothing was copied into $assets."
    fi
  done
fi
run mkdir -p "$dist/publish" "$assets"
copied=0
for name in $names; do
  file="$out/$name.json"
  if [ "$dry" = 0 ] && [ ! -f "$file" ]; then continue; fi
  run cp "$file" "$dist/publish/$name.json"
  run cp "$file" "$assets/$name.json"
  copied=$((copied + 1))
done
[ "$copied" -gt 0 ] || fail "publish.mjs wrote no catalog"
if [ "$dry" = 0 ]; then
  for name in $names; do
    if [ -f "$assets/$name.json" ]; then
      echo "$assets/$name.json: $(jq -r .topic_count "$assets/$name.json") topics"
    fi
  done
fi
echo "The earlier files are in $backup. Build Nexus again to bundle the new catalogs. Don't commit them."
