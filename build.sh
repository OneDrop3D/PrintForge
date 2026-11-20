#!/bin/zsh
# Construit OneDrop PrintForge, le signe avec une identité STABLE puis (option --installer) le copie dans /Applications.
# Sans signature stable, macOS voit une nouvelle application à chaque compilation et redemande l'accès au réseau local
# (recherche de l'imprimante, PrusaLink).
cd "${0:A:h}" || exit 1
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"
cargo tauri build --bundles app || exit 1
APP="src-tauri/target/release/bundle/macos/OneDrop PrintForge.app"
ID="${ONEDROP3D_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | grep -oE '"(Multibook|ClickDeck)[^"]*"' | head -1 | tr -d '"')}"
if [ -n "$ID" ]; then
  codesign --force --deep --sign "$ID" --identifier fr.onedrop3d.printforge "$APP" 2>&1 | tail -2
  # signature ratée (trousseau verrouillé…) : ne pas installer une application que macOS refuserait
  codesign --verify --deep --strict "$APP" || { echo "!! signature invalide : rien n'est installé"; exit 1; }
  echo "signature : $ID (vérifiée)"
else
  echo "!! aucune identité de signature : signature ad hoc, macOS redemandera l'accès au réseau local (voir multibook tauri/tools/certificat-local.sh)"
fi
# « mac » (Publieur) : l'installeur .dmg que les gens téléchargent, rangé dans dist/ avec sa version.
if [ "$1" = "mac" ]; then
  V=$(python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
  mkdir -p dist && rm -f "dist/OneDrop-PrintForge-$V-mac-arm64.dmg"
  hdiutil create -quiet -volname "OneDrop PrintForge" -srcfolder "$APP" -format UDZO "dist/OneDrop-PrintForge-$V-mac-arm64.dmg" || exit 1
  echo "installeur : dist/OneDrop-PrintForge-$V-mac-arm64.dmg"
fi
if [ "$1" = "--installer" ]; then
  osascript -e 'tell application "OneDrop PrintForge" to quit' >/dev/null 2>&1; sleep 1
  rm -rf "/Applications/OneDrop PrintForge.app" && cp -R "$APP" /Applications/ && echo "installé dans /Applications"
fi
# Le disque est petit : comme pour OneDrop3D et Multibook, seule l'application finie reste.
# « --garder » : compilation suivante plus rapide.
if [[ " $* " != *" --garder "* ]]; then
  rm -rf src-tauri/target/release/{deps,build,incremental,.fingerprint} src-tauri/target/debug
fi
