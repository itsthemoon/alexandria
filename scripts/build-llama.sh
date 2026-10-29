#!/usr/bin/env bash
set -euo pipefail

PLATFORM=${1:?usage: build-llama.sh <platform> <dest>}
DEST=${2:?usage: build-llama.sh <platform> <dest>}
COMMIT=1b4554eff124a8f0033bc09b3f708a5e51cbda15
JOBS=$( (nproc || sysctl -n hw.logicalcpu || echo 4) 2>/dev/null | head -1)

SRC=${LLAMA_SRC:-$(mktemp -d)/llama.cpp}
if [ ! -d "$SRC/.git" ]; then
  git init -q "$SRC"
  git -C "$SRC" remote add origin https://github.com/ggml-org/llama.cpp
fi
git -C "$SRC" fetch -q --depth 1 origin "$COMMIT"
git -C "$SRC" checkout -q --force "$COMMIT"
test "$(git -C "$SRC" rev-parse HEAD)" = "$COMMIT"

COMMON=(
  -DCMAKE_BUILD_TYPE=Release
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF
  -DLLAMA_OPENSSL=OFF -DGGML_RPC=OFF
  -DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_CONNECT=OFF
)

case "$PLATFORM" in
  macos-arm64)
    EXTRA=(-DBUILD_SHARED_LIBS=OFF -DGGML_METAL_EMBED_LIBRARY=ON -DGGML_NATIVE=OFF -DGGML_CPU_ARM_ARCH=armv8.4-a+fp16+dotprod
           -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=13.3) ;;
  macos-x64)
    EXTRA=(-DBUILD_SHARED_LIBS=OFF -DGGML_METAL=OFF -DGGML_NATIVE=OFF
           -DCMAKE_OSX_ARCHITECTURES=x86_64 -DCMAKE_OSX_DEPLOYMENT_TARGET=13.3) ;;
  linux-x64)
    EXTRA=(-DGGML_BACKEND_DL=ON -DGGML_NATIVE=OFF -DGGML_CPU_ALL_VARIANTS=ON
           -DCMAKE_INSTALL_RPATH='$ORIGIN' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON) ;;
  windows-x64)
    EXTRA=(-G "Ninja Multi-Config" -DCMAKE_TOOLCHAIN_FILE=cmake/x64-windows-llvm.cmake
           -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON
           -DGGML_OPENMP=OFF) ;;
  windows-arm64)
    EXTRA=(-G "Ninja Multi-Config" -DCMAKE_TOOLCHAIN_FILE=cmake/arm64-windows-llvm.cmake
           -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=OFF
           -DGGML_OPENMP=OFF) ;;
  *) echo "unknown platform: $PLATFORM" >&2; exit 2 ;;
esac

(cd "$SRC" && cmake -S . -B "build-$PLATFORM" "${COMMON[@]}" "${EXTRA[@]}" >/dev/null)
if [ "${PLATFORM%%-*}" = macos ]; then
  (cd "$SRC" && cmake --build "build-$PLATFORM" --config Release --target llama-completion -j "$JOBS")
else
  (cd "$SRC" && cmake --build "build-$PLATFORM" --config Release -j "$JOBS")
fi

BIN="$SRC/build-$PLATFORM/bin"
[ -d "$BIN/Release" ] && BIN="$BIN/Release"
mkdir -p "$DEST"
case "$PLATFORM" in
  macos-*) cp "$BIN/llama-completion" "$DEST/" ;;
  linux-x64) cp "$BIN/llama-completion" "$DEST/"; cp -P "$BIN"/*.so* "$DEST/" ;;
  windows-*) cp "$BIN/llama-completion.exe" "$DEST/"; cp "$BIN"/*.dll "$DEST/" ;;
esac
echo "built llama.cpp $COMMIT for $PLATFORM into $DEST"
ls -la "$DEST"
