#!/bin/sh
# Keeps the script-plugin conformance vectors in step between the two repositories.
# The vectors' home is rattleboard-plugins/conformance/script/; the prelude's home is
# crates/rattleboard-core/src/script_prelude.js. This copies each to the other side:
#
#   scripts/sync-conformance.sh [path/to/rattleboard-plugins]   (default: ../rattleboard-plugins)
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
plugins=${1:-"$here/../rattleboard-plugins"}
[ -d "$plugins/conformance/script" ] || { echo "No conformance/script in $plugins" >&2; exit 1; }
cp "$here/crates/rattleboard-core/src/script_prelude.js" "$plugins/conformance/script-prelude.js"
rm -rf "$here/crates/rattleboard-core/tests/conformance"
mkdir -p "$here/crates/rattleboard-core/tests/conformance"
cp -R "$plugins/conformance/script" "$here/crates/rattleboard-core/tests/conformance/script"
cp "$plugins/conformance/script-prelude.js" "$here/crates/rattleboard-core/tests/conformance/script-prelude.js"
echo "Synced $(ls "$plugins/conformance/script" | wc -l | tr -d ' ') vectors and the prelude."
