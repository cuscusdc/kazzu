#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist/ios"
VERSION="0.0.0"
TARGET=""

usage() {
  echo "uso: scripts/build-ios.sh --prepare | --target device|simulator [--version X.Y.Z-ci.N]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --prepare) TARGET=prepare; shift ;;
    --target) TARGET="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    *) usage ;;
  esac
done

BASE="${VERSION%%-ci.*}"
BUILD="${VERSION#*-ci.}"
if [ "$BUILD" = "$VERSION" ]; then BUILD=1; fi
if [ -z "$BASE" ]; then BASE=0.0.0; fi

case "$TARGET" in
  prepare)
    mkdir -p "$OUT"
    xcodegen generate --spec "$ROOT/ios/project.yml" --use-cache
    ;;
  device)
    mkdir -p "$OUT/device"
    xcodebuild -project "$ROOT/ios/Kazzu.xcodeproj" -scheme Kazzu \
      -configuration Release -destination 'generic/platform=iOS' \
      -archivePath "$OUT/Kazzu.xcarchive" \
      MARKETING_VERSION="$BASE" CURRENT_PROJECT_VERSION="$BUILD" \
      CODE_SIGNING_ALLOWED=NO archive
    rm -rf "$OUT/Payload"
    mkdir -p "$OUT/Payload"
    cp -R "$OUT/Kazzu.xcarchive/Products/Applications/Kazzu.app" "$OUT/Payload/Kazzu.app"
    (cd "$OUT" && zip -qry "$ROOT/dist/ios/Kazzu-device.ipa" Payload)
    ;;
  simulator)
    mkdir -p "$OUT/simulator"
    xcodebuild -project "$ROOT/ios/Kazzu.xcodeproj" -scheme Kazzu \
      -configuration Release -destination 'generic/platform=iOS Simulator' \
      -derivedDataPath "$OUT/derived" \
      MARKETING_VERSION="$BASE" CURRENT_PROJECT_VERSION="$BUILD" \
      CODE_SIGNING_ALLOWED=NO build
    APP="$(find "$OUT/derived/Build/Products" -name 'Kazzu.app' -type d -path '*Release-iphonesimulator*' | head -1)"
    if [ -z "$APP" ]; then
      echo "app do simulador não encontrado" >&2
      exit 1
    fi
    cp -R "$APP" "$OUT/simulator/Kazzu.app"
    ;;
  *)
    usage
    ;;
esac
