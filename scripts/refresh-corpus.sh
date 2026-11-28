#!/usr/bin/env bash
# Rebuild testdata/corpus: generate the source files, then let LibreOffice open, recalculate and
# save each one, so the committed files carry cached values from an independent engine.
# Needs LibreOffice (soffice). Run from the repository root.
set -euo pipefail

src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT
cargo run -q -p speedsheet-core --example gen_corpus -- "$src"
mkdir -p testdata/corpus
# Separate LibreOffice profile so a running LibreOffice does not swallow the conversion, set to
# always recalculate xlsx files on load (by default it keeps the cached values, here none).
mkdir -p "$src/lo-profile/user"
cat >"$src/lo-profile/user/registrymodifications.xcu" <<'XCU'
<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Calc/Formula/Load"><prop oor:name="OOXMLRecalcMode" oor:op="fuse"><value>0</value></prop></item>
</oor:items>
XCU
profile="file://$src/lo-profile"
for f in "$src"/*.xlsx; do
  soffice -env:UserInstallation="$profile" --headless --calc --convert-to 'xlsx:Calc MS Excel 2007 XML' \
    --outdir testdata/corpus "$f" >/dev/null
done
# Macro files are kept as the generator wrote them: the point is to check that the macros survive.
cp "$src"/*.xlsm testdata/corpus/
ls -l testdata/corpus
