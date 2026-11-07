#!/usr/bin/env bash
# Assembles the GitHub Pages site in _site/: the start page and findings page from site/,
# the book under /book and the rustdoc under /api.
#
#   scripts/build-site.sh && python3 -m http.server -d _site 8000
set -euo pipefail
cd "$(dirname "$0")/.."

mdbook build book
cargo doc --no-deps --workspace

rm -rf _site
mkdir -p _site
cp -r site/. _site/
cp -r book/build _site/book
cp -r target/doc/. _site/api/          # keeps site/api/index.html, which rustdoc does not write
cp book/src/studies/bulk-transfer/generated/findings.json _site/findings/data.json
touch _site/.nojekyll
echo "site in _site/"
