#!/bin/sh
set -eu

project=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
version=1.19.1
cache=${XDG_CACHE_HOME:-"$HOME/.cache"}/kazzu-ui-preview
viewer="$cache/$version/slint-viewer"

if [ -n "${SLINT_VIEWER:-}" ]; then
    viewer=$SLINT_VIEWER
elif [ ! -x "$viewer" ]; then
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64)
            asset=slint-viewer-x86_64-unknown-linux-gnu.tar.gz
            digest=ae41f8e0b61c7f746d78f7240de9dd110bc9e86606cf43dcebfebd6e6ed8af1e
            ;;
        Linux:aarch64)
            asset=slint-viewer-aarch64-unknown-linux-gnu.tar.gz
            digest=1387d16c0716f39ccb000fc5294527df6124b438aa0f42ce2989f5da7e72e5c1
            ;;
        *)
            printf '%s\n' "Baixe o Slint Viewer $version e informe o executável em SLINT_VIEWER." >&2
            exit 1
            ;;
    esac

    printf '%s\n' "Baixando o Slint Viewer $version pré-compilado…" >&2
    mkdir -p "$cache"
    tmp=$(mktemp -d "$cache/download.XXXXXX")
    trap 'rm -rf -- "$tmp"' EXIT
    trap 'exit 130' HUP INT TERM
    curl --fail --location --silent --show-error --retry 2 --connect-timeout 10 --max-time 120 \
        "https://github.com/slint-ui/slint/releases/download/v$version/$asset" -o "$tmp/$asset"
    printf '%s  %s\n' "$digest" "$tmp/$asset" | sha256sum --check --status
    tar -xzf "$tmp/$asset" -C "$tmp"
    mkdir -p "$cache/$version"
    mv -f -- "$tmp/slint-viewer/slint-viewer" "$viewer"
fi

if [ "$#" -eq 0 ]; then
    set -- --backend winit-skia --auto-reload
fi
exec "$viewer" "$@" "$project"
