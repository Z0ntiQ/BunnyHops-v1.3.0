#!/usr/bin/env bash
# Сборка оконного приложения Bunny Hops (Tauri v2) под текущую ОС.
# Требования: Rust (stable), tauri-cli (cargo install tauri-cli --version "^2"),
# frontend уже собран (trunk build в ../frontend), для Linux — системные зависимости webkit2gtk.
set -euo pipefail
cd "$(dirname "$0")/../app"

# 1. SPA-дистрибутив (окно грузит тот же dist)
(cd frontend && trunk build --release)

# 2. Релизная сборка + бандл под текущую платформу
(cd desktop && cargo tauri build)

echo
echo "Готово. Артефакты:"
case "$(uname -s)" in
  Darwin) echo "  macOS:  desktop/target/release/bundle/macos/Bunny Hops.app";;
  Linux)  echo "  Linux:  desktop/target/release/bundle/deb/ и bundle/rpm/";;
esac
