#!/usr/bin/env bash
# Download counts from GitHub, per release and file: the real number of
# downloads (the site only sees clicks). Update bundles (.app.tar.gz) are
# the macOS updater's; on Windows the updater downloads the same setup.exe
# people do, so its count includes updates.
set -euo pipefail

gh api --paginate repos/HiteshRohira/ReVLC/releases \
  --jq '.[] | .tag_name as $tag | .assets[] | [$tag, .name, .download_count] | @tsv' |
  awk -F'\t' '
    { printf "%-16s %-44s %8d\n", $1, $2, $3; total[$1] += $3 }
    END { print ""; for (t in total) printf "%-16s %-44s %8d\n", t, "(all files)", total[t] }
  '
