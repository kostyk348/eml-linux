#!/bin/sh
# Install emlinit as a user dinit service.
set -e
DIR="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$HOME/.config/dinit.d" "$HOME/.local/state/emlinit"
cp "$DIR/deploy/dinit-emlinit" "$HOME/.config/dinit.d/emlinit"
echo "installed: ~/.config/dinit.d/emlinit"
echo "start with: dinitctl start emlinit"
echo "logs:       $DIR/target/release/emllog --spool $HOME/.local/state/emlinit/spool tail"
