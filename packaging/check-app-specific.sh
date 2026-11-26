#!/usr/bin/env bash
# App-specific checks retained alongside shared packaging lint.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

bash packaging/server/install-test.sh
if grep -rniE 'photoshop|psd|photocraft|vectorcraft|illustrator|adobe|figma' packaging/ .github/ \
    --exclude=packaging-lint.yml --exclude=check-app-specific.sh; then
  echo "::error::package metadata mentions another product (see above)" >&2
  exit 1
fi
if ! grep -qx 'Name=Monet' packaging/linux/app.monet.design.desktop \
  || ! grep -qx 'Exec=monet %U' packaging/linux/app.monet.design.desktop; then
  echo "::error::Linux desktop entry is not the Monet app" >&2
  exit 1
fi

python3 - <<'PYEOF'
import yaml
manifests = []
for name in ("app.monet.design.yml", "app.monet.design.bundle.yml"):
    with open("packaging/linux/flatpak/" + name) as source:
        manifest = yaml.safe_load(source)
    assert manifest["id"] == "app.monet.design" and manifest["command"] == "monet", name
    manifests.append(manifest)
for key in ("runtime", "runtime-version", "sdk", "finish-args"):
    assert manifests[0][key] == manifests[1][key], f"Flatpak manifests disagree on {key}"
print("Monet package identity ok")
PYEOF
