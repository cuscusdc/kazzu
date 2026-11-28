#!/usr/bin/env bash
# Compila o Kazzu para Android e gera um APK assinado (sem Gradle, sem Android Studio).
#
# Passos: cargo (cdylib para o alvo Android, NDK como linker) -> aapt2 (manifesto + ícones) -> adiciona libkazzu_app.so
# (sem compressão, alinhada a 16 KB) -> zipalign -> apksigner.
#
# Uso:
#   scripts/build-apk.sh [--abi arm64-v8a|x86_64|armeabi-v7a]... [--version 0.1.0-alpha.1] [--debug-sign] [--out DIR]
#
# Ambiente (padrões entre parênteses):
#   ANDROID_HOME                 SDK ($HOME/.local/share/android-sdk, ou $HOME/Android/Sdk)
#   ANDROID_NDK_HOME             NDK (a versão mais nova em $ANDROID_HOME/ndk)
#   CARGO_TARGET_DIR             (<repo>/target/android)
#   KAZZU_KEYSTORE               keystore de release (~/.android/kazzu-release.jks)
#   KAZZU_KEYSTORE_PASS_FILE     arquivo com a senha (~/.android/kazzu-release.pass); a senha nunca vai para o log
#   KAZZU_KEY_ALIAS              alias da chave (kazzu)
#   KAZZU_SIGNER_SHA256          impressão digital esperada do certificado oficial, opcional
# --debug-sign: assina com uma chave descartável (só para testes/CI sem segredo); NÃO serve para publicar.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"

ABIS=()
VERSION=""
DEBUG_SIGN=0
OUT="$REPO/target/android/apk"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --abi) ABIS+=("$2"); shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --debug-sign) DEBUG_SIGN=1; shift ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "opção desconhecida: $1" >&2; exit 2 ;;
  esac
done
[[ ${#ABIS[@]} -gt 0 ]] || ABIS=(arm64-v8a)
if [[ -z "$VERSION" ]]; then
  VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
fi

# --- SDK / NDK ------------------------------------------------------------------------------------------------
if [[ -z "${ANDROID_HOME:-}" ]]; then
  for c in "$HOME/.local/share/android-sdk" "$HOME/Android/Sdk"; do [[ -d "$c" ]] && ANDROID_HOME="$c" && break; done
fi
[[ -d "${ANDROID_HOME:-}" ]] || { echo "ANDROID_HOME não definido e SDK não encontrado (ver docs/ANDROID.md)" >&2; exit 1; }
export ANDROID_HOME
export ANDROID_SDK_ROOT="$ANDROID_HOME"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  ANDROID_NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/*/ 2>/dev/null | sort -V | tail -1)"
  ANDROID_NDK_HOME="${ANDROID_NDK_HOME%/}"
fi
[[ -d "${ANDROID_NDK_HOME:-}" ]] || { echo "NDK não encontrado em $ANDROID_HOME/ndk" >&2; exit 1; }
export ANDROID_NDK_HOME ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
BT="$(ls -d "$ANDROID_HOME"/build-tools/*/ | sort -V | tail -1)"; BT="${BT%/}"
PLATFORM_JAR="$(ls -d "$ANDROID_HOME"/platforms/android-*/ | sort -V | tail -1)android.jar"
# cmake + ninja do SDK (o libopus é compilado junto; ver docs/ANDROID.md)
CMAKE_BIN="$(ls -d "$ANDROID_HOME"/cmake/*/bin 2>/dev/null | sort -V | tail -1 || true)"
[[ -n "$CMAKE_BIN" ]] && export PATH="$CMAKE_BIN:$PATH"
command -v cmake >/dev/null || { echo "cmake não encontrado (sdkmanager \"cmake;3.31.6\")" >&2; exit 1; }
export CMAKE_GENERATOR=Ninja
TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
MIN_SDK=26
# O build do libopus (cmake) lê CARGO_NDK_*; ANDROID_PLATFORM não pode ser definido (o backend do Slint o usa para achar o android.jar).
export CARGO_NDK_ANDROID_PLATFORM="android-$MIN_SDK" CARGO_NDK_ANDROID_STL=c++_static
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/android}"

# --- versão numérica: 0.1.0-alpha.1 -> 100 + 1 ; 0.1.0 -> 100 + 99 (final > alpha) --------------------------------
IFS='.-' read -r V_MAJ V_MIN V_PAT V_PRE V_PRE_N <<<"$VERSION"
if [[ -n "${V_PRE:-}" ]]; then BUILD=${V_PRE_N:-1}; else BUILD=99; fi
VERSION_CODE=$((V_MAJ * 1000000 + V_MIN * 10000 + V_PAT * 100 + BUILD))

# --- assinatura -----------------------------------------------------------------------------------------------
mkdir -p "$OUT"
KEY_ALIAS="${KAZZU_KEY_ALIAS:-kazzu}"
EXPECTED_CERT=""
if [[ "${DEBUG_SIGN}" -eq 0 && -n "${KAZZU_SIGNER_SHA256:-}" ]]; then
  EXPECTED_CERT="$(printf '%s' "$KAZZU_SIGNER_SHA256" | tr -d '[:space:]:' | tr '[:upper:]' '[:lower:]')"
  [[ "$EXPECTED_CERT" =~ ^[[:xdigit:]]{64}$ ]] || {
    echo "impressão digital SHA-256 do certificado inválida" >&2
    exit 2
  }
fi
if [[ $DEBUG_SIGN -eq 1 ]]; then
  KS="$REPO/target/android/debug.jks"; PASS_FILE="$REPO/target/android/debug.pass"; KEY_ALIAS=kazzu-debug
  if [[ ! -f "$KS" ]]; then
    umask 077; mkdir -p "$(dirname "$KS")"
    head -c 24 /dev/urandom | base64 > "$PASS_FILE"
    keytool -genkeypair -keystore "$KS" -storetype PKCS12 -storepass:file "$PASS_FILE" -keypass:file "$PASS_FILE" \
      -alias "$KEY_ALIAS" -keyalg RSA -keysize 2048 -validity 3650 -dname "CN=Kazzu debug" >/dev/null
  fi
else
  KS="${KAZZU_KEYSTORE:-$HOME/.android/kazzu-release.jks}"
  PASS_FILE="${KAZZU_KEYSTORE_PASS_FILE:-$HOME/.android/kazzu-release.pass}"
  [[ -f "$KS" && -f "$PASS_FILE" ]] || { echo "keystore/senha de release não encontrados ($KS). Veja docs/ANDROID.md ou use --debug-sign." >&2; exit 1; }
fi

# --- ícones/recursos ------------------------------------------------------------------------------------------
WORK="$REPO/target/android/work"
rm -rf "$WORK"; mkdir -p "$WORK"
"$BT/aapt2" compile --dir android/res -o "$WORK/res.zip"

# Compila a pequena ponte do Android e a converte para DEX uma vez por execução.
mkdir -p "$WORK/java" "$WORK/dex"
javac --release 8 -classpath "$PLATFORM_JAR" -d "$WORK/java" android/src/app/kazzu/player/*.java
mapfile -d '' JAVA_CLASSES < <(find "$WORK/java" -name '*.class' -print0)
"$BT/d8" --lib "$PLATFORM_JAR" --min-api "$MIN_SDK" --output "$WORK/dex" "${JAVA_CLASSES[@]}"

for ABI in "${ABIS[@]}"; do
  case "$ABI" in
    arm64-v8a)   TRIPLE=aarch64-linux-android;    CLANG_PREFIX=aarch64-linux-android ;;
    armeabi-v7a) TRIPLE=armv7-linux-androideabi;  CLANG_PREFIX=armv7a-linux-androideabi ;;
    x86_64)      TRIPLE=x86_64-linux-android;     CLANG_PREFIX=x86_64-linux-android ;;
    *) echo "ABI não suportada: $ABI" >&2; exit 2 ;;
  esac
  UNDER="${TRIPLE//-/_}"; UPPER="$(echo "$UNDER" | tr a-z A-Z)"
  CLANG="$TOOLCHAIN/${CLANG_PREFIX}${MIN_SDK}-clang"
  export "CC_$UNDER=$CLANG" "CXX_$UNDER=${CLANG}++" "AR_$UNDER=$TOOLCHAIN/llvm-ar" "CARGO_TARGET_${UPPER}_LINKER=$CLANG"
  export "CARGO_TARGET_${UPPER}_AR=$TOOLCHAIN/llvm-ar"

  echo "==> cargo: kazzu_app ($TRIPLE, release)"
  # `--crate-type cdylib` só aqui: o crate continua `lib` no desktop. Sem a feature `demo` (gera dados de teste).
  cargo rustc --config "profile.release.package.kazzu-app.opt-level=${KAZZU_APP_OPT_LEVEL:-1}" -j "${KAZZU_BUILD_JOBS:-1}" -p kazzu-app --lib --release ${CI:+--locked} --no-default-features --target "$TRIPLE" \
    --config 'profile.release.strip="symbols"' --crate-type cdylib
  SO="$CARGO_TARGET_DIR/$TRIPLE/release/libkazzu_app.so"
  [[ -f "$SO" ]] || { echo "libkazzu_app.so não gerada" >&2; exit 1; }

  echo "==> empacotando $ABI"
  BASE="$WORK/base-$ABI.apk"
  "$BT/aapt2" link -o "$BASE" -I "$PLATFORM_JAR" --manifest android/AndroidManifest.xml \
    --min-sdk-version "$MIN_SDK" --target-sdk-version 34 --version-code "$VERSION_CODE" --version-name "$VERSION" \
    -R "$WORK/res.zip" --auto-add-overlay
  # biblioteca nativa sem compressão (extractNativeLibs=false); python3 só para escrever o zip
  python3 -I - "$BASE" "$SO" "$ABI" "$WORK/dex/classes.dex" <<'PY'
import sys, zipfile
apk, so, abi, dex = sys.argv[1:5]
with zipfile.ZipFile(apk, "a") as z:
    z.write(dex, "classes.dex", compress_type=zipfile.ZIP_DEFLATED)
    z.write(so, f"lib/{abi}/libkazzu_app.so", compress_type=zipfile.ZIP_STORED)
PY
  ALIGNED="$WORK/aligned-$ABI.apk"
  "$BT/zipalign" -f -P 16 4 "$BASE" "$ALIGNED"
  APK="$OUT/kazzu-$VERSION-$ABI.apk"
  "$BT/apksigner" sign --ks "$KS" --ks-key-alias "$KEY_ALIAS" --ks-pass "file:$PASS_FILE" \
    --min-sdk-version "$MIN_SDK" --v4-signing-enabled false --out "$APK" "$ALIGNED"
  "$BT/apksigner" verify --min-sdk-version "$MIN_SDK" "$APK"
  if [[ -n "$EXPECTED_CERT" ]]; then
    ACTUAL_CERT="$(LC_ALL=C "$BT/apksigner" verify --print-certs --min-sdk-version "$MIN_SDK" "$APK" |
      awk -F ': ' '/^Signer #[0-9]+ certificate SHA-256 digest:/ { print tolower($2) }')"
    [[ "$ACTUAL_CERT" == "$EXPECTED_CERT" ]] || {
      echo "certificado do APK não corresponde à impressão digital oficial" >&2
      exit 1
    }
  fi
  (cd "$OUT" && sha256sum "$(basename "$APK")" > "$(basename "$APK").sha256")
  echo "==> $APK ($(du -h "$APK" | cut -f1))"
done
