#!/usr/bin/env bash
# Downloads one file: `scripts/download.sh <url> <file>`.
#
# With GH_TOKEN set and gh installed (CI), a GitHub URL goes through gh, which sends the
# token: GitHub serves logged-in downloads when it times out logged-out ones. curl can't do
# this itself, as GitHub's downloads redirect to other hosts and curl drops the token there.
#   github.com/<owner>/<repo>/releases/download/<tag>/<file>   gh release download
#   github.com/<owner>/<repo>/archive/<ref>.tar.gz             gh api .../tarball/<ref>
#   raw.githubusercontent.com/<owner>/<repo>/<ref>/<path>      gh api .../contents/<path>
# Anything else, and everything without a token (a developer's machine), goes through curl.
# Each way tries 5 times, 10 seconds apart.
set -euo pipefail
url="${1:?usage: $0 <url> <file>}"
out="${2:?usage: $0 <url> <file>}"
mkdir -p "$(dirname "$out")"

# One try of the download through gh. Only for a URL gh_can_fetch accepts.
through_gh() {
    local rest owner repo
    case "$url" in
        https://github.com/*/releases/download/*)
            rest="${url#https://github.com/}"
            owner="${rest%%/*}" rest="${rest#*/}"
            repo="${rest%%/*}" rest="${rest#*/releases/download/}"
            gh release download "${rest%/*}" -R "$owner/$repo" --pattern "${rest##*/}" \
                --output "$out" --clobber
            ;;
        https://github.com/*/archive/*.tar.gz)
            rest="${url#https://github.com/}"
            owner="${rest%%/*}" rest="${rest#*/}"
            repo="${rest%%/*}" rest="${rest#*/archive/}"
            gh api "repos/$owner/$repo/tarball/${rest%.tar.gz}" > "$out"
            ;;
        https://raw.githubusercontent.com/*)
            rest="${url#https://raw.githubusercontent.com/}"
            owner="${rest%%/*}" rest="${rest#*/}"
            repo="${rest%%/*}" rest="${rest#*/}"
            # The ref is the first part of the rest; this repo's refs have no slash.
            gh api -H 'Accept: application/vnd.github.raw' \
                "repos/$owner/$repo/contents/${rest#*/}?ref=${rest%%/*}" > "$out"
            ;;
    esac
}

gh_can_fetch() {
    case "$url" in
        https://github.com/*/releases/download/* | https://github.com/*/archive/*.tar.gz \
            | https://raw.githubusercontent.com/*) return 0 ;;
        *) return 1 ;;
    esac
}

if [ -n "${GH_TOKEN:-}" ] && command -v gh > /dev/null && gh_can_fetch; then
    for try in 1 2 3 4 5; do
        through_gh && exit 0
        echo "download.sh: try $try of $url failed" >&2
        [ "$try" = 5 ] && exit 1
        sleep 10
    done
fi
curl -fsSL --retry 5 --retry-delay 10 --retry-all-errors -o "$out" "$url"
